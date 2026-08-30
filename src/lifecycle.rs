//! Meter lifecycle events: installation, exchange, and retirement.
//!
//! ## Legal basis
//!
//! - **WiM Gerätewechsel-Dokumentation**: Messstellenbetreiber must document meter exchanges.
//! - **§14 MsbG**: Meter data must be available at supply handover.
//! - **BDEW GPKE**: Zählerwechsel triggers a Sonderablesung (INSRPT PID 23003).
//!
//! ## Why this matters for billing
//!
//! When a meter is replaced mid-period, two separate readings exist:
//! - **Old meter**: last reading before exchange
//! - **New meter**: first reading after exchange
//!
//! The `MeterExchangeEvent` anchors both readings to a single point in time,
//! enabling correct Mehr-/Mindermengensaldo calculation and billing continuity.

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime};

use crate::ids::MeloId;
use crate::reading::{Anomaly, LastgangConfig, MeterReading, consumption_between};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Status of a physical meter at a delivery point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum MeterStatus {
    /// Meter is installed and in service.
    #[default]
    Active,
    /// Meter has been removed / decommissioned.
    Removed,
    /// Meter is installed but not yet commissioned.
    Pending,
    /// Meter was tested in lab; now deployed to a MeLo.
    Deployed,
}

impl MeterStatus {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 4] = [Self::Active, Self::Removed, Self::Pending, Self::Deployed];

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Removed => "REMOVED",
            Self::Pending => "PENDING",
            Self::Deployed => "DEPLOYED",
        }
    }

    /// `true` when the meter can currently produce readings.
    #[must_use]
    pub const fn is_in_service(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Type of lifecycle event affecting a physical meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum MeterLifecycleEventType {
    /// Initial installation at a delivery point.
    Installed,
    /// Meter replaced with a new unit at the same delivery point.
    Replaced,
    /// Meter removed without replacement (end of supply).
    Removed,
    /// Meter firmware or calibration updated (SMGW update, new Eichung).
    Updated,
    /// Meter sealed / calibration renewed (Eichung, §28 MessEG).
    Recalibrated,
}

impl MeterLifecycleEventType {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Installed,
        Self::Replaced,
        Self::Removed,
        Self::Updated,
        Self::Recalibrated,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "INSTALLED",
            Self::Replaced => "REPLACED",
            Self::Removed => "REMOVED",
            Self::Updated => "UPDATED",
            Self::Recalibrated => "RECALIBRATED",
        }
    }

    /// `true` when the event changes which physical register is counting, so a
    /// reading either side of it cannot be differenced against the other.
    ///
    /// [`MeterExchangeEvent`] is the type that pairs the two readings across
    /// such a break.
    #[must_use]
    pub const fn breaks_register_continuity(self) -> bool {
        matches!(self, Self::Installed | Self::Replaced | Self::Removed)
    }
}

crate::codes::string_codes! {
    MeterStatus;
    MeterLifecycleEventType;
}

/// A lifecycle event for a physical meter.
///
/// Stored as an immutable audit log — never updated, only appended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterLifecycleEvent {
    /// Unique event identifier.
    pub event_id: String,
    /// Meter serial number (Zählernummer / Gerätenummer).
    pub meter_serial: String,
    /// Messlokations-ID of the delivery point — see [`MeloId`].
    pub melo_id: MeloId,
    /// Type of lifecycle event.
    pub event_type: MeterLifecycleEventType,
    /// When the event occurred (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub occurred_at: OffsetDateTime,
    /// The meter reading at the time of the event (kWh).
    /// `None` when not applicable (e.g., firmware update).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub reading: Option<Decimal>,
    /// OBIS code of the reading (when `reading` is set).
    pub obis_code: Option<crate::obis::ObisCode>,
    /// Free-text reason / operator note.
    pub reason: Option<String>,
    /// BDEW PID that triggered this event (e.g., 23003 for Zählerwechsel).
    pub triggered_by_pid: Option<u32>,
}

/// A meter exchange event: the old meter is replaced by a new one.
///
/// ## Relationship to [`crate::reading`]
///
/// The helpers below difference register readings, which is
/// [`reading::consumption_between`](crate::reading::consumption_between)'s job
/// in general. They stay here because an exchange is the one case that needs
/// **two** registers: the difference cannot be taken across the boundary,
/// because the new meter starts over. What this type adds is the pairing of the
/// old meter's final reading with the new one's first.
///
/// An exchange is also the likeliest explanation for a backwards step that
/// [`reading::to_lastgang`](crate::reading::to_lastgang) refuses to read as a
/// register wrap — see [`AnomalyKind::ImplausibleRollover`].
///
/// [`AnomalyKind::ImplausibleRollover`]: crate::reading::AnomalyKind::ImplausibleRollover
///
/// ## Billing continuity
///
/// The pair `(old_reading, new_reading)` at `exchange_at` enables seamless
/// computation of consumption across the exchange boundary:
///
/// ```text
/// consumption_before = old_reading_kwh − period_start_reading
/// consumption_after  = period_end_reading − new_first_reading
/// total_period       = consumption_before + consumption_after
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterExchangeEvent {
    /// Unique exchange identifier.
    pub exchange_id: String,
    /// The MeLo where the exchange took place — see [`MeloId`].
    pub melo_id: MeloId,
    /// Serial number of the removed meter.
    pub old_meter_serial: String,
    /// Final reading of the old meter (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub old_final_reading: Decimal,
    /// Serial number of the newly installed meter.
    pub new_meter_serial: String,
    /// First reading of the new meter (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub new_first_reading: Decimal,
    /// Date and time of the exchange (UTC).
    ///
    /// The Berlin calendar day a billing period aligns on is derived from it —
    /// [`exchange_date`](Self::exchange_date) — rather than stored beside it. A
    /// second copy of one fact is a second thing to keep in step, and the
    /// timestamp is the one that decides: 23:30 UTC on 14 June is already
    /// 15 June in Berlin, and a hand-filled date field said otherwise.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub exchange_at: OffsetDateTime,
    /// BDEW PID that triggered this exchange (typically 23003).
    pub triggered_by_pid: Option<u32>,
    /// INSRPT process ID that reported this exchange.
    pub insrpt_process_id: Option<String>,
    /// Technician or system that performed the exchange.
    pub performed_by: Option<String>,
}

impl MeterExchangeEvent {
    /// The **Berlin calendar day** the exchange falls on, for billing-period
    /// alignment.
    ///
    /// Derived from [`exchange_at`](Self::exchange_at) through
    /// [`crate::calendar::local_day`], so it is right across both DST
    /// transitions and cannot drift out of step with the instant.
    #[must_use]
    pub fn exchange_date(&self) -> Date {
        crate::calendar::local_day(self.exchange_at)
    }

    /// The old meter's final reading, as a [`MeterReading`] at the exchange
    /// instant.
    #[must_use]
    pub fn old_final(&self) -> MeterReading {
        MeterReading::measured(self.exchange_at, self.old_final_reading)
    }

    /// The new meter's first reading, as a [`MeterReading`] at the exchange
    /// instant.
    #[must_use]
    pub fn new_first(&self) -> MeterReading {
        MeterReading::measured(self.exchange_at, self.new_first_reading)
    }

    /// Consumption on the **old** meter from `period_start` to the exchange.
    ///
    /// # Errors
    ///
    /// Returns the [`Anomaly`] when no honest difference exists — see
    /// [`consumption_between`].
    ///
    /// The `Result` matters: clamping a backwards step to zero would bill
    /// **0 kWh** for the whole pre-exchange span of a Jahresabrechnung whose
    /// old register had wrapped, silently. Reconstructing the wrap is what
    /// [`LastgangConfig::register_digits`] is for.
    pub fn consumption_old_meter(
        &self,
        period_start: &MeterReading,
        config: &LastgangConfig,
    ) -> Result<Decimal, Anomaly> {
        consumption_between(period_start, &self.old_final(), config)
    }

    /// Consumption on the **new** meter from the exchange to `period_end`.
    ///
    /// # Errors
    ///
    /// As [`consumption_old_meter`](Self::consumption_old_meter).
    pub fn consumption_new_meter(
        &self,
        period_end: &MeterReading,
        config: &LastgangConfig,
    ) -> Result<Decimal, Anomaly> {
        consumption_between(&self.new_first(), period_end, config)
    }

    /// Total consumption across the exchange boundary.
    ///
    /// The two meters are differenced separately and summed — the readings
    /// cannot be subtracted across the boundary, because the new register
    /// starts over. That is the whole reason this type exists.
    ///
    /// # Errors
    ///
    /// The first [`Anomaly`] either side produces; a failure on one meter makes
    /// the total unusable whatever the other one says.
    pub fn total_consumption(
        &self,
        period_start: &MeterReading,
        period_end: &MeterReading,
        config: &LastgangConfig,
    ) -> Result<Decimal, Anomaly> {
        Ok(self.consumption_old_meter(period_start, config)?
            + self.consumption_new_meter(period_end, config)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::{date, datetime};

    fn make_exchange() -> MeterExchangeEvent {
        MeterExchangeEvent {
            exchange_id: "EX-001".to_owned(),
            melo_id: "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap(),
            old_meter_serial: "OLD-1234".to_owned(),
            old_final_reading: dec!(12500),
            new_meter_serial: "NEW-5678".to_owned(),
            new_first_reading: dec!(0), // new meter starts at 0
            exchange_at: datetime!(2026-06-15 8:00 UTC),
            triggered_by_pid: Some(23003),
            insrpt_process_id: None,
            performed_by: Some("MSB-9900357000004".to_owned()),
        }
    }

    fn start(value: Decimal) -> MeterReading {
        MeterReading::measured(datetime!(2026-06-01 0:00 UTC), value)
    }

    fn end(value: Decimal) -> MeterReading {
        MeterReading::measured(datetime!(2026-06-30 0:00 UTC), value)
    }

    #[test]
    fn consumption_split_across_exchange() {
        let ex = make_exchange();
        let cfg = LastgangConfig::default();
        // Old meter: 12 000 on 1 June, 12 500 at the exchange → 500 kWh.
        // New meter: 0 at the exchange, 800 on 30 June → 800 kWh.
        assert_eq!(
            ex.consumption_old_meter(&start(dec!(12000)), &cfg),
            Ok(dec!(500))
        );
        assert_eq!(
            ex.consumption_new_meter(&end(dec!(800)), &cfg),
            Ok(dec!(800))
        );
        assert_eq!(
            ex.total_consumption(&start(dec!(12000)), &end(dec!(800)), &cfg),
            Ok(dec!(1300))
        );
    }

    /// A register that wraps during the period is reconstructed, not clamped:
    /// flooring the difference at zero would bill nothing at all for the whole
    /// pre-exchange span.
    #[test]
    fn a_wrapped_old_register_is_reconstructed_not_zeroed() {
        use crate::reading::AnomalyKind;

        let mut ex = make_exchange();
        ex.old_final_reading = dec!(300); // six-digit register wrapped past 999 999
        let period_start = start(dec!(999_500));

        // Without a register width there is no honest answer, and the caller is
        // told so instead of being handed a zero.
        let blind = ex.consumption_old_meter(&period_start, &LastgangConfig::default());
        assert_eq!(
            blind.unwrap_err().kind,
            AnomalyKind::BackwardsWithoutRegisterWidth
        );

        // With one, the wrap is reconstructed: (1 000 000 − 999 500) + 300.
        let cfg = LastgangConfig::default().with_register_digits(6);
        assert_eq!(ex.consumption_old_meter(&period_start, &cfg), Ok(dec!(800)));
    }

    /// A transposed pair — period start after the final reading — is an error,
    /// not zero consumption.
    #[test]
    fn a_backwards_reading_is_reported_rather_than_zeroed() {
        let ex = make_exchange();
        let err = ex
            .consumption_old_meter(&start(dec!(13000)), &LastgangConfig::default())
            .unwrap_err();
        assert!(err.to_string().contains("register decreased"), "{err}");
    }

    /// A failure on either meter makes the total unusable.
    #[test]
    fn the_total_fails_when_either_side_does() {
        let ex = make_exchange();
        let cfg = LastgangConfig::default();
        assert!(
            ex.total_consumption(&start(dec!(13000)), &end(dec!(800)), &cfg)
                .is_err()
        );
        assert!(
            ex.total_consumption(&start(dec!(12000)), &end(dec!(-5)), &cfg)
                .is_err()
        );
    }

    /// The exchange day is derived from the instant, not stored beside it —
    /// 22:30 UTC on 14 June is already 15 June in Berlin, and a hand-filled
    /// date field is free to disagree.
    #[test]
    fn the_exchange_day_is_derived_from_the_instant() {
        let mut ex = make_exchange();
        assert_eq!(ex.exchange_date(), date!(2026 - 06 - 15));

        ex.exchange_at = datetime!(2026-06-14 22:30 UTC); // 00:30 CEST on the 15th
        assert_eq!(
            ex.exchange_date(),
            date!(2026 - 06 - 15),
            "the Berlin calendar day, not the UTC one"
        );
        assert_eq!(ex.exchange_at.date(), date!(2026 - 06 - 14));
    }

    /// The lifecycle enums carry the same code vocabulary as every other
    /// domain enum here, so a stored status reads back as itself.
    #[test]
    fn lifecycle_codes_round_trip() {
        assert_eq!(MeterStatus::ALL.len(), MeterStatus::CODES.len());
        for (v, code) in MeterStatus::ALL.iter().zip(MeterStatus::CODES) {
            assert_eq!(v.as_str(), *code);
            assert_eq!(v.to_string(), *code);
            assert_eq!(&v.to_string().parse::<MeterStatus>().unwrap(), v);
        }
        assert_eq!(
            MeterLifecycleEventType::ALL.len(),
            MeterLifecycleEventType::CODES.len()
        );
        for (v, code) in MeterLifecycleEventType::ALL
            .iter()
            .zip(MeterLifecycleEventType::CODES)
        {
            assert_eq!(v.as_str(), *code);
            assert_eq!(
                &v.to_string().parse::<MeterLifecycleEventType>().unwrap(),
                v
            );
        }

        assert_eq!(
            " active ".parse::<MeterStatus>().unwrap(),
            MeterStatus::Active
        );
        assert!("GARBAGE".parse::<MeterStatus>().is_err());
        assert!(MeterStatus::Active.is_in_service());
        assert!(!MeterStatus::Removed.is_in_service());

        // Only the three events that swap the physical register break the
        // difference a Lastgang is built from.
        assert!(MeterLifecycleEventType::Replaced.breaks_register_continuity());
        assert!(!MeterLifecycleEventType::Updated.breaks_register_continuity());
        assert!(!MeterLifecycleEventType::Recalibrated.breaks_register_continuity());
    }
}

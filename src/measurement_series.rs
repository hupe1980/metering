//! Named, annotated measurement series — grouping of intervals with provenance metadata.
//!
//! A [`MeasurementSeries`] is the semantic container that wraps a `Vec<MeterInterval>`
//! with the context required to explain every value:
//! - **who** measured it (MaLo, MeLo, meter serial, OBIS register)
//! - **how** it was produced (source, ingestion method, quality)
//! - **why** it exists (purpose, process reference)
//!
//! ## Relationship to domain objects
//!
//! ```text
//! MeasurementSeries
//!   ├── source_info: MeasurementSource   ← where data came from
//!   ├── measurement_point: Option<MeasurementPoint>  ← full location context
//!   ├── resolution: IntervalResolution   ← expected interval length
//!   ├── intervals: Vec<MeterInterval>    ← the actual data
//!   └── provenance: Vec<ProvenanceEntry> ← correction/substitution audit trail
//! ```
//!
//! ## Explainability
//!
//! Every stored interval should answer *"where did this value come from?"*.
//! `MeasurementSeries` carries the answer at the series level, each
//! [`MeterInterval::quality`](crate::MeterInterval::quality) at the interval
//! level, and [`crate::substitute::SubstituteEntry`] for values that were
//! computed rather than measured.
//!
//! ## § 60 Abs. 6 MsbG is a deletion duty, not a retention mandate
//!
//! It is commonly read as its opposite, so it is worth quoting:
//!
//! > Der Messstellenbetreiber muss personenbezogene Messwerte … **löschen oder
//! > … anonymisieren**, sobald für seine Aufgabenwahrnehmung eine Speicherung
//! > personenbezogener Messwerte nicht mehr erforderlich ist, **spätestens
//! > jedoch nach drei Jahren** ab dem Schluss des Kalenderjahres, in dem der
//! > jeweilige Messwert erhoben wurde …
//!
//! Three years is a **ceiling**, and the operative trigger is earlier still —
//! as soon as the data is no longer needed. A provenance trail is therefore
//! something to keep only as long as the values it explains, and
//! [`ProvenanceEventType::Anonymised`] exists because erasure is itself an
//! event the trail has to record.
//!
//! Retention obligations that do point the other way — the Eichrecht
//! documentation duties, and any longer period the Bundesnetzagentur sets under
//! the reservation in the same sentence — are outside this crate.
//!
//! ## Legal basis
//!
//! - **§ 60 Abs. 1 MsbG** — the MSB prepares and transmits the data of §§ 55–59.
//! - **§ 60 Abs. 6 MsbG** — deletion or anonymisation, as above.
//! - **BDEW MSCONS AHB** — each MSCONS time series is one named series per OBIS
//!   code, which is what this type models.

use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::aggregation_rule::VirtualMeterKind;
use crate::ids::{BdewCode, MaloId, MeloId};
use crate::interval::{MeterInterval, QualityFlag};
use crate::obis::ObisCode;
use crate::resolution::IntervalResolution;

// ── MeasurementSource ─────────────────────────────────────────────────────────

/// The origin of a measurement series — how the data entered the system.
///
/// Stored per series (not per interval) since all intervals in one MSCONS
/// message share the same ingestion source. The same holds for a charging
/// session: one [`split_session`](crate::split_session) call produces one
/// series, and every slot in it came from the same record.
///
/// ## Provenance is not billability
///
/// Whether a value may be billed is decided by each interval's
/// [`QualityFlag`] — only `Faulty` and `Unknown` block billing — never by
/// where the series came from: a manual entry after a dispute and a GGV
/// virtual-meter result are billed every day. A predicate on this type would
/// be a second, contradictory notion of billability.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum MeasurementSource {
    /// Received via EDIFACT MSCONS (standard MaKo pipeline).
    ///
    /// The canonical source for DSO-metered customers. Follows the
    /// market-communication → master-data → metering webhook pipeline.
    Mscons {
        /// MSCONS Prüfidentifikator (e.g. 13005).
        pid: u32,
        /// EDIFACT message reference.
        message_ref: Option<String>,
        /// Marktpartner-ID of the sending NB/MSB — see [`BdewCode`].
        sender_mp_id: BdewCode,
    },

    /// iMSys / SMGW direct push — bypasses EDIFACT pipeline.
    ///
    /// Used for §41a EnWG dynamic tariffs where MSCONS round-trip adds latency.
    SmgwDirectPush {
        /// SMGW device ID.
        device_id: String,
        /// Session ID for idempotency.
        session_id: String,
    },

    /// Manual entry by an operator.
    ///
    /// Used for corrections after meter replacement or dispute resolution.
    ManualEntry {
        /// Operator identifier (user ID or name).
        operator_id: String,
        /// Reason for manual entry.
        reason: String,
    },

    /// A substitute value generated by [`crate::substitute`].
    ///
    /// Typically triggered by gap detection (V01) in the validation engine.
    AutoSubstitute {
        /// Substitute method used.
        method: crate::substitute::SubstituteMethod,
        /// Reason for substitution.
        reason: crate::substitute::SubstitutionReason,
    },

    /// A retroactive correction, applied when an earlier value was found wrong.
    RetroactiveCorrection {
        /// Reference to the correction record in the caller's own system.
        ///
        /// A `String`, like every other external reference in this enum: the
        /// shape of a consumer's primary key is not this crate's to decide,
        /// and nothing here constructs, parses or validates it.
        correction_ref: String,
        /// Who applied the correction.
        corrected_by: String,
    },

    /// Derived by [`crate::compute_virtual_meter`] from other series.
    VirtualMeter {
        /// Which rule produced it.
        ///
        /// A [`VirtualMeterKind`] rather than free text, so a typo in a
        /// hand-built record is a compile error rather than a stale value.
        rule: VirtualMeterKind,
        /// Source MaLo / MeLo IDs that contributed to this series.
        source_ids: Vec<String>,
    },

    /// Redispatch 2.0 time-series import (PIDs 13020–13023, 13026).
    ///
    /// Ausfallarbeit, meteorological data, and other Redispatch quantities.
    RedispatchImport {
        /// MSCONS PID (13020–13023, 13026).
        pid: u32,
        /// Activation ID or process reference.
        activation_ref: Option<String>,
    },

    /// A charging session's **Charge Detail Record** — one total for the whole
    /// session, placed on the metering grid by
    /// [`split_session`](crate::split_session).
    ///
    /// The energy is measured: a CDR is the difference of the charge point's
    /// register at the start and the end of the transaction, and under the
    /// Eichrecht it is signed. What is *not* measured is where inside the
    /// session it flowed, so every slot the session did not fill exactly comes
    /// back [`Estimated`](QualityFlag::Estimated). Distinguishing that from
    /// [`ClockAlignedMeterValue`](Self::ClockAlignedMeterValue) is the point of
    /// having two variants: a supplier settling this energy needs to know
    /// which quarter-hours were read and which were divided.
    ChargeDetailRecord {
        /// The CDR's identifier in the caller's own system or in OCPI.
        cdr_id: String,
        /// The EVSE the session ran on (ISO 15118 / eMI3), when known.
        evse_id: Option<String>,
    },

    /// **Clock-aligned meter values** from a charge point — OCPP
    /// `MeterValues` sampled on the `ClockAlignedDataInterval`.
    ///
    /// The readings land on the settlement grid's own boundaries, so the slots
    /// between two of them are measured rather than inferred. This is the
    /// source a charge point should be configured to produce wherever the
    /// energy is going to be settled per quarter-hour.
    ClockAlignedMeterValue {
        /// The OCPP transaction the readings belong to.
        transaction_id: String,
        /// The EVSE they were taken at (ISO 15118 / eMI3), when known.
        evse_id: Option<String>,
    },

    /// A device's **own log** — a heat pump, a battery, a wallbox submeter.
    ///
    /// Not the Messstellenbetreiber's meter, and not necessarily an
    /// eichrechtskonform one: a device register is fit for allocating and for
    /// diagnostics, and the moment it is used to bill a third party the
    /// Eichrecht applies to it like any other measuring instrument. The
    /// variant exists so that a series carrying such values is never mistaken
    /// for one that came from the MSB.
    DeviceLog {
        /// The device the log came from.
        device_id: String,
        /// Which of the device's registers, when it has more than one.
        register: Option<String>,
    },
}

impl MeasurementSource {
    /// Short human-readable label (German).
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Mscons { .. } => "MSCONS",
            Self::SmgwDirectPush { .. } => "SMGW-Direktpush",
            Self::ManualEntry { .. } => "Manuelle Eingabe",
            Self::AutoSubstitute { .. } => "Automatischer Ersatzwert",
            Self::RetroactiveCorrection { .. } => "Nachträgliche Korrektur",
            Self::VirtualMeter { .. } => "Virtueller Zähler",
            Self::RedispatchImport { .. } => "Redispatch 2.0",
            Self::ChargeDetailRecord { .. } => "Ladevorgang (CDR)",
            Self::ClockAlignedMeterValue { .. } => "Ladepunkt-Zählwert",
            Self::DeviceLog { .. } => "Gerätelog",
        }
    }
}

// ── ProvenanceEntry ───────────────────────────────────────────────────────────

/// An immutable audit record for a change applied to a series or interval.
///
/// Provenance entries are append-only: they record what happened and when, and
/// never overwrite an earlier entry. The trail lives exactly as long as the
/// values it explains — see the [module docs](self#-60-abs-6-msbg-is-a-deletion-duty-not-a-retention-mandate).
///
/// `occurred_at` is always supplied by the caller: this crate never reads the
/// system clock, so two series built from the same inputs compare equal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProvenanceEntry {
    /// When this event occurred (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub occurred_at: OffsetDateTime,
    /// What kind of event this was.
    pub event_type: ProvenanceEventType,
    /// Who or what triggered this event.
    pub actor: String,
    /// Free-text note for regulatory audit trail.
    pub note: Option<String>,
}

/// Type of provenance event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum ProvenanceEventType {
    /// Initial ingest of the series.
    Ingested,
    /// Quality assessment run (V01–V11 validation).
    QualityAssessed,
    /// Gap detected and substitute value generated.
    SubstituteGenerated,
    /// Retroactive correction applied.
    Corrected,
    /// Archive to cold tier (Iceberg/S3).
    Archived,
    /// GDPR erasure request applied.
    Anonymised,
}

impl ProvenanceEventType {
    /// Every event type, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Ingested,
        Self::QualityAssessed,
        Self::SubstituteGenerated,
        Self::Corrected,
        Self::Archived,
        Self::Anonymised,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ingested => "INGESTED",
            Self::QualityAssessed => "QUALITY_ASSESSED",
            Self::SubstituteGenerated => "SUBSTITUTE_GENERATED",
            Self::Corrected => "CORRECTED",
            Self::Archived => "ARCHIVED",
            Self::Anonymised => "ANONYMISED",
        }
    }
}

crate::codes::string_codes! {
    ProvenanceEventType;
}

// ── MeasurementSeries ─────────────────────────────────────────────────────────

/// A named, annotated time series of meter intervals with full provenance.
///
/// This is the richest representation of meter data in the `metering` crate.
/// It combines the context required to explain where every value came from.
///
/// ## Usage
///
/// For most processing (aggregation, validation, resampling), use `Vec<MeterInterval>`
/// directly. Use `MeasurementSeries` at system boundaries where the full context
/// must be preserved: persistence, tool responses, and ERP handoffs.
///
/// ## Relationship to `MeasurementPoint`
///
/// `MeasurementPoint` describes the **physical and regulatory binding** (MaLo,
/// MeLo, OBIS, MarktRolle). `MeasurementSeries` describes the **data series**
/// with its provenance. One `MeasurementPoint` can produce many
/// `MeasurementSeries` (e.g. one per MSCONS delivery).
///
/// ## Construction is deterministic
///
/// [`new`](Self::new) takes the ingest timestamp rather than reading the system
/// clock, so two series built from equal inputs *are* equal — which is what
/// makes storage round-trip tests possible. See the crate-level
/// **Determinism** section.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeasurementSeries {
    /// Marktlokations-ID, check-digit validated at the parse — see
    /// [`MaloId`].
    pub malo_id: MaloId,

    /// Messlokations-ID (if available) — see [`MeloId`].
    pub melo_id: Option<MeloId>,

    /// OBIS code identifying this measurement channel.
    pub obis_code: Option<ObisCode>,

    /// Expected interval resolution for this series.
    ///
    /// Derived from `obis_code.default_resolution()` when not explicitly set.
    pub resolution: Option<IntervalResolution>,

    /// How the data entered the system.
    pub source: MeasurementSource,

    /// The interval data, ordered by `from` ascending.
    pub intervals: Vec<MeterInterval>,

    /// Audit trail for this series (ordered chronologically).
    pub provenance: Vec<ProvenanceEntry>,
}

impl MeasurementSeries {
    /// Construct a new series from intervals, recording an `Ingested`
    /// provenance entry stamped `ingested_at`.
    ///
    /// The timestamp is a parameter, not a clock read: a domain library that
    /// samples ambient state cannot be replayed, cached or tested by equality.
    /// Callers holding a clock pass `OffsetDateTime::now_utc()`; callers
    /// replaying an archive pass the archived timestamp, and get the archived
    /// series back.
    #[must_use]
    pub fn new(
        malo_id: MaloId,
        obis_code: Option<ObisCode>,
        intervals: Vec<MeterInterval>,
        source: MeasurementSource,
        ingested_at: OffsetDateTime,
    ) -> Self {
        let resolution = obis_code.and_then(|o| o.default_resolution());
        let actor = source.label().to_owned();
        Self {
            malo_id,
            melo_id: None,
            obis_code,
            resolution,
            source,
            intervals,
            provenance: vec![ProvenanceEntry {
                occurred_at: ingested_at,
                event_type: ProvenanceEventType::Ingested,
                actor,
                note: None,
            }],
        }
    }

    /// Attach the MeLo-ID (builder style).
    #[must_use]
    pub fn with_melo_id(mut self, melo_id: MeloId) -> Self {
        self.melo_id = Some(melo_id);
        self
    }

    /// Override the resolution inferred from the OBIS code (builder style).
    #[must_use]
    pub const fn with_resolution(mut self, resolution: IntervalResolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// Worst quality flag across all intervals, or
    /// [`QualityFlag::Unknown`] when the series is empty.
    ///
    /// Computed on demand rather than cached in a field: a stored copy is a
    /// second source of truth that any direct mutation of `intervals` silently
    /// invalidates, and the ranking is a single pass over data already in
    /// memory. Persisting layers should derive it the same way instead of
    /// storing it.
    #[must_use]
    pub fn worst_quality(&self) -> QualityFlag {
        QualityFlag::worst_of(self.intervals.iter().map(|iv| iv.quality))
    }

    /// `true` when at least one interval may not be billed — its quality is
    /// `Faulty` or `Unknown`.
    #[must_use]
    pub fn has_unbillable_intervals(&self) -> bool {
        self.intervals.iter().any(|iv| !iv.quality.is_billable())
    }

    /// Number of intervals in this series.
    #[must_use]
    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    /// `true` when the series has no intervals (e.g. empty MSCONS delivery).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// Total energy (kWh) across all billable intervals.
    #[must_use]
    pub fn total(&self) -> rust_decimal::Decimal {
        self.intervals
            .iter()
            .filter(|iv| iv.quality.is_billable())
            .map(|iv| iv.value)
            .sum()
    }

    /// Earliest interval start in this series.
    #[must_use]
    pub fn period_from(&self) -> Option<OffsetDateTime> {
        self.intervals.iter().map(|iv| iv.from).min()
    }

    /// Latest interval end in this series.
    #[must_use]
    pub fn period_to(&self) -> Option<OffsetDateTime> {
        self.intervals.iter().map(|iv| iv.to).max()
    }

    /// Append a provenance entry stamped `occurred_at`.
    ///
    /// As with [`new`](Self::new), the timestamp is supplied rather than read
    /// from the system clock.
    pub fn record_event(
        &mut self,
        event_type: ProvenanceEventType,
        actor: impl Into<String>,
        occurred_at: OffsetDateTime,
    ) {
        self.provenance.push(ProvenanceEntry {
            occurred_at,
            event_type,
            actor: actor.into(),
            note: None,
        });
    }

    /// Append a provenance entry with an audit note.
    pub fn record_event_with_note(
        &mut self,
        event_type: ProvenanceEventType,
        actor: impl Into<String>,
        occurred_at: OffsetDateTime,
        note: impl Into<String>,
    ) {
        self.provenance.push(ProvenanceEntry {
            occurred_at,
            event_type,
            actor: actor.into(),
            note: Some(note.into()),
        });
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use crate::obis::ObisCode;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    const INGEST: OffsetDateTime = datetime!(2026-01-02 09:30 UTC);

    fn make_interval(from: OffsetDateTime, kwh: rust_decimal::Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    fn malo() -> MaloId {
        "51238696781".parse().unwrap()
    }

    fn mscons() -> MeasurementSource {
        MeasurementSource::Mscons {
            pid: 13005,
            message_ref: None,
            sender_mp_id: "9900357000004".parse().unwrap(),
        }
    }

    #[test]
    fn worst_quality_is_derived_from_the_intervals() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let mut intervals = vec![
            make_interval(base, dec!(1.0)),
            make_interval(base + Duration::minutes(15), dec!(1.0)),
        ];
        intervals[1].quality = QualityFlag::Estimated;

        let mut series = MeasurementSeries::new(
            malo(),
            Some(ObisCode::STROM_BEZUG_TOTAL),
            intervals,
            mscons(),
            INGEST,
        );
        assert_eq!(series.worst_quality(), QualityFlag::Estimated);
        assert_eq!(series.interval_count(), 2);

        // The former cached field could go stale against a direct mutation of
        // `intervals`; a derivation cannot.
        series.intervals.push({
            let mut iv = make_interval(base + Duration::minutes(30), dec!(1.0));
            iv.quality = QualityFlag::Faulty;
            iv
        });
        assert_eq!(series.worst_quality(), QualityFlag::Faulty);
        assert!(series.has_unbillable_intervals());
    }

    #[test]
    fn empty_series_has_unknown_worst_quality() {
        let series = MeasurementSeries::new(malo(), None, vec![], mscons(), INGEST);
        assert_eq!(series.worst_quality(), QualityFlag::Unknown);
        assert!(
            !series.has_unbillable_intervals(),
            "no intervals, no faults"
        );
    }

    /// The reason construction takes a timestamp: identical inputs must produce
    /// identical values, or no storage layer can write a round-trip test.
    #[test]
    fn construction_is_deterministic() {
        let intervals = vec![make_interval(datetime!(2026-01-01 0:00 UTC), dec!(1.0))];
        let a = MeasurementSeries::new(malo(), None, intervals.clone(), mscons(), INGEST);
        let b = MeasurementSeries::new(malo(), None, intervals, mscons(), INGEST);
        assert_eq!(a, b, "equal inputs must give equal series");
        assert_eq!(a.provenance[0].occurred_at, INGEST);
        assert_eq!(a.provenance[0].event_type, ProvenanceEventType::Ingested);
    }

    #[test]
    fn recorded_events_append_in_the_order_given() {
        let mut series = MeasurementSeries::new(malo(), None, vec![], mscons(), INGEST);
        let later = INGEST + Duration::hours(3);
        series.record_event(ProvenanceEventType::QualityAssessed, "validator", later);
        series.record_event_with_note(
            ProvenanceEventType::Corrected,
            "ops-001",
            later + Duration::hours(1),
            "Zählerwechsel nachgetragen",
        );

        assert_eq!(series.provenance.len(), 3);
        assert_eq!(series.provenance[1].occurred_at, later);
        assert_eq!(series.provenance[1].actor, "validator");
        assert_eq!(
            series.provenance[2].note.as_deref(),
            Some("Zählerwechsel nachgetragen")
        );
    }

    #[test]
    fn total_kwh_sums_billable_only() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let mut intervals = vec![
            make_interval(base, dec!(2.0)),
            make_interval(base + Duration::minutes(15), dec!(1.0)),
        ];
        intervals[1].quality = QualityFlag::Faulty; // not billable

        let series = MeasurementSeries::new(
            malo(),
            None,
            intervals,
            MeasurementSource::ManualEntry {
                operator_id: "ops-001".to_owned(),
                reason: "test".to_owned(),
            },
            INGEST,
        );
        assert_eq!(series.total(), dec!(2.0)); // only Measured interval
    }

    #[test]
    fn period_bounds_from_intervals() {
        let base = datetime!(2026-06-01 0:00 UTC);
        let intervals = vec![
            make_interval(base, dec!(1.0)),
            make_interval(base + Duration::minutes(15), dec!(1.0)),
        ];
        let series = MeasurementSeries::new(
            malo(),
            None,
            intervals,
            MeasurementSource::SmgwDirectPush {
                device_id: "SMGW-001".to_owned(),
                session_id: "sess-001".to_owned(),
            },
            INGEST,
        );
        assert_eq!(series.period_from(), Some(base));
        assert_eq!(series.period_to(), Some(base + Duration::minutes(30)));
    }

    #[test]
    fn empty_series_reports_correctly() {
        let series = MeasurementSeries::new(
            malo(),
            None,
            vec![],
            MeasurementSource::AutoSubstitute {
                method: crate::substitute::SubstituteMethod::ZeroFill,
                reason: crate::substitute::SubstitutionReason::NoMeasurementAvailable,
            },
            INGEST,
        );
        assert!(series.is_empty());
        assert_eq!(series.total(), rust_decimal::Decimal::ZERO);
        assert!(series.period_from().is_none());
    }

    #[test]
    fn builders_set_optional_context() {
        let series = MeasurementSeries::new(malo(), None, vec![], mscons(), INGEST)
            .with_melo_id("DE00056266802AO6G56M11SN51G21M24S".parse().unwrap())
            .with_resolution(IntervalResolution::Hour);
        assert_eq!(
            series.melo_id.as_ref().map(MeloId::as_str),
            Some("DE00056266802AO6G56M11SN51G21M24S")
        );
        assert_eq!(series.resolution, Some(IntervalResolution::Hour));
    }

    #[test]
    fn source_labels_non_empty() {
        let sources = [
            mscons(),
            MeasurementSource::SmgwDirectPush {
                device_id: "d".into(),
                session_id: "s".into(),
            },
            MeasurementSource::ManualEntry {
                operator_id: "o".into(),
                reason: "r".into(),
            },
            MeasurementSource::AutoSubstitute {
                method: crate::substitute::SubstituteMethod::LinearInterpolation,
                reason: crate::substitute::SubstitutionReason::MeterFault,
            },
            MeasurementSource::RetroactiveCorrection {
                correction_ref: "corr-2026-0001".to_owned(),
                corrected_by: "op".into(),
            },
            MeasurementSource::VirtualMeter {
                rule: VirtualMeterKind::Sum,
                source_ids: vec![],
            },
            MeasurementSource::RedispatchImport {
                pid: 13022,
                activation_ref: None,
            },
        ];
        for s in &sources {
            assert!(!s.label().is_empty());
        }
    }

    #[test]
    fn obis_default_resolution_wires_into_series() {
        let series = MeasurementSeries::new(
            malo(),
            Some(ObisCode::STROM_BEZUG_TOTAL),
            vec![],
            mscons(),
            INGEST,
        );
        assert_eq!(series.resolution, Some(IntervalResolution::QuarterHour));
    }
}

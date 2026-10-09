//! The Messtyp code list — SLP, RLM, iMSys.
//!
//! Master data of the Messlokation (§ 2 MsbG), stated by the caller — never
//! inferred from a series, whose interval length says only how data was delivered.

/// Metering type (Messtyp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Messtyp {
    /// Standard load profile — daily or coarser aggregates, no interval series.
    Slp,
    /// Registrierende Lastgangmessung — an interval series, 15 to 60 minutes (§ 2 MsbG).
    Rlm,
    /// Intelligentes Messsystem behind a Smart-Meter-Gateway (§ 2 Satz 1 Nr. 7 MsbG), quarter-hourly.
    IMsys,
}

impl Messtyp {
    /// Every Messtyp, in declaration order.
    pub const ALL: [Self; 3] = [Self::Slp, Self::Rlm, Self::IMsys];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Slp => "SLP",
            Self::Rlm => "RLM",
            Self::IMsys => "I_MSYS",
        }
    }

    /// `true` when the Messtyp supports Spitzenleistung (peak demand) billing.
    #[must_use]
    pub fn supports_spitzenleistung(&self) -> bool {
        matches!(self, Messtyp::Rlm | Messtyp::IMsys)
    }

    /// `true` when the Messtyp can serve a § 41a EnWG dynamic tariff (needs quarter-hour values).
    #[must_use]
    pub fn supports_dynamic_tariff(&self) -> bool {
        matches!(self, Messtyp::IMsys)
    }
}

crate::ids::codes::string_codes! {
    Messtyp;
}

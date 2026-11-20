//! From which day the Festlegung governs a site — Tenorziffern 8 and 9.
//!
//! [`geltung`] takes the latest of three days: [`WIRKSAM_AB`] (Tenorziffer 8);
//! without the operators' consent, the day after [`EINVERSTAENDNIS_BIS`]
//! (9 a); for the Pauschaloption, the first day of the month after the EU's
//! state-aid approval of § 19 Abs. 3c EEG (9 b). The approval date is the
//! caller's; without it the Pauschaloption is refused. The result names the
//! rule that set the day — on a tie the more particular one: approval, then
//! consent, then Bekanntgabe.
//!
//! The site's Zuordnung (Tenorziffer 5) is the caller's date too: a first
//! Zuordnung may start inside a month (Anlage 1 Kap. 11, a Rumpfmonat), a
//! *Wechsel* on the first. Take the later of the two days.
//!
//! ```rust
//! use metering::eeg::mispel::geltung::{geltung, Grund, Optionsart, Umstaende};
//! use metering::eeg::mispel::pauschal::Zeitraum;
//! use time::macros::date;
//!
//! let g = geltung(
//!     Optionsart::Pauschal,
//!     Umstaende { einverstaendnis: true, genehmigung: Some(date!(2027 - 01 - 14)) },
//! )?;
//! assert_eq!((g.ab, g.grund), (date!(2027 - 02 - 01), Grund::Genehmigung));
//!
//! // The first Pauschaloption year is a Rumpfjahr, the next one whole.
//! assert_eq!(
//!     Zeitraum::ab(2027, g.ab),
//!     Some(Zeitraum::Rumpfjahr { erster_tag: date!(2027 - 02 - 01), letzter_tag: date!(2027 - 12 - 31) }),
//! );
//! assert_eq!(Zeitraum::ab(2028, g.ab), Some(Zeitraum::Kalenderjahr(2028)));
//! # Ok::<(), metering::eeg::EegError>(())
//! ```

use std::fmt;

use time::macros::date;
use time::{Date, Month};

use crate::eeg::EegError;

/// The day the Festlegung takes effect: **21.10.2026**.
///
/// Tenorziffer 8 ties the effect to the Bekanntgabe. The Amtsblatt of the
/// Bundesnetzagentur 19/2026 of 07.10.2026 (Vfg. Nr. 73/2026) states when that
/// is: *"Diese Entscheidung gilt gem. § 73 Abs. 1a EnWG i. V. m. § 85 Abs. 3 EEG
/// und § 62 Abs. 3 EnFG mit dem Tag als zugestellt, an dem seit dem Tag der
/// Bekanntmachung im vorliegenden Amtsblatt zwei Wochen verstrichen sind."* The
/// adoption date, 01.10.2026, is not the effect date.
pub const WIRKSAM_AB: Date = date!(2026 - 10 - 21);

/// The last day the Festlegung needs the operators' consent: **30.09.2027**.
///
/// Tenorziffer 9 a: *"Die Regelungen der Festlegung sind bis zum 30.09.2027 nur
/// anwendbar, wenn die Geltendmachung der Ansprüche nach den Tenorziffern 1 bis
/// 4 und die Zuordnung nach Tenorziffer 5 im Einverständnis mit dem jeweiligen
/// Netzbetreiber und Messstellenbetreiber erfolgen."* It binds both options.
pub const EINVERSTAENDNIS_BIS: Date = date!(2027 - 09 - 30);

/// The first day without the consent condition.
const OHNE_EINVERSTAENDNIS_AB: Date = match EINVERSTAENDNIS_BIS.next_day() {
    Some(d) => d,
    None => unreachable!(),
};

/// Which of the two options the site is settled under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Optionsart {
    /// Anlage 1 — the Abgrenzungsoption. No state-aid condition.
    Abgrenzung,
    /// Anlage 2 — the Pauschaloption, bound by Tenorziffer 9 b.
    Pauschal,
}

/// What only the caller knows. Both fields are always written: there is no
/// `Default`, because neither has one the Festlegung would accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Umstaende {
    /// Whether the Netzbetreiber **and** the Messstellenbetreiber consented
    /// (Tenorziffer 9 a). Irrelevant from 01.10.2027.
    pub einverstaendnis: bool,
    /// The day of the EU's state-aid approval of § 19 Abs. 3c EEG, if given
    /// (Tenorziffer 9 b). Read only for the Pauschaloption.
    pub genehmigung: Option<Date>,
}

/// The rule that set the first governed day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Grund {
    /// Tenorziffer 8: the Bekanntgabe, [`WIRKSAM_AB`].
    Bekanntgabe,
    /// Tenorziffer 9 a: no consent, so the day after [`EINVERSTAENDNIS_BIS`].
    Einverstaendnis,
    /// Tenorziffer 9 b: the first day of the month after the state-aid approval.
    Genehmigung,
}

impl fmt::Display for Grund {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bekanntgabe => "MiSpeL Tenorziffer 8: effect from the Bekanntgabe",
            Self::Einverstaendnis => "MiSpeL Tenorziffer 9 a: without consent, from 01.10.2027",
            Self::Genehmigung => {
                "MiSpeL Tenorziffer 9 b: from the month after the state-aid approval"
            }
        })
    }
}

/// The first day the Festlegung may govern the site, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Geltung {
    /// The first governed day (inclusive).
    pub ab: Date,
    /// The rule that set it.
    pub grund: Grund,
}

/// The first day `option` may govern a site, given what the caller knows.
///
/// The latest of: [`WIRKSAM_AB`]; without consent, 01.10.2027; for the
/// Pauschaloption, the first day of the month after the approval — so an
/// approval on 01.03.2027 gives 01.04.2027.
///
/// # Errors
///
/// [`EegError::NoStateAidApproval`] for the Pauschaloption without an approval
/// date; [`EegError::Calendar`] when the approval's month has no successor in
/// the supported calendar.
pub fn geltung(option: Optionsart, umstaende: Umstaende) -> Result<Geltung, EegError> {
    let mut g = Geltung {
        ab: WIRKSAM_AB,
        grund: Grund::Bekanntgabe,
    };
    // `>=`: on a tie the rule applied later — the more particular — is named.
    let mut later = |ab: Date, grund: Grund| {
        if ab >= g.ab {
            g = Geltung { ab, grund };
        }
    };
    if !umstaende.einverstaendnis {
        later(OHNE_EINVERSTAENDNIS_AB, Grund::Einverstaendnis);
    }
    if option == Optionsart::Pauschal {
        let genehmigung = umstaende.genehmigung.ok_or(EegError::NoStateAidApproval)?;
        later(erster_des_folgemonats(genehmigung)?, Grund::Genehmigung);
    }
    Ok(g)
}

/// The first day of the month after `tag`'s month.
fn erster_des_folgemonats(tag: Date) -> Result<Date, EegError> {
    let (year, month) = match tag.month() {
        Month::December => (tag.year() + 1, Month::January),
        m => (tag.year(), m.next()),
    };
    Date::from_calendar_date(year, month, 1).map_err(|_| EegError::Calendar)
}

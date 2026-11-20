//! What the source does not define is a parameter, never a default.
//!
//! Each type below holds a value the caller must choose — an operator's
//! table, a rounding, a consent, a reading of a silent rule. A `Default` would
//! answer for the caller with a plausible number. Adding one fails here.

use std::marker::PhantomData;

use metering::billing::reactive::ReactiveLimit;
use metering::eeg::mispel::geltung::Umstaende;
use metering::gas::conversion::{G685Rounding, GasConversionParams, ZustandszahlParams};
use metering::grid::para14a::{Para14aConfig, Verursachungsregel};
use metering::grid::power_quality::En50160Limits;

/// Whether `T: Default`, decided at compile time without naming the bound at
/// the call site: an inherent associated constant applies only where its
/// `where` clause holds, and otherwise the blanket trait's constant does.
struct Probe<T>(PhantomData<T>);

trait NoDefault {
    const IMPLEMENTS_DEFAULT: bool = false;
}
impl<T> NoDefault for Probe<T> {}

impl<T: Default> Probe<T> {
    const IMPLEMENTS_DEFAULT: bool = true;
}

macro_rules! implements_default {
    ($t:ty) => {
        Probe::<$t>::IMPLEMENTS_DEFAULT
    };
}

#[test]
fn parameter_types_have_no_default() {
    let found: Vec<&str> = [
        ("Umstaende", implements_default!(Umstaende)),
        ("Para14aConfig", implements_default!(Para14aConfig)),
        (
            "Verursachungsregel",
            implements_default!(Verursachungsregel),
        ),
        (
            "GasConversionParams",
            implements_default!(GasConversionParams),
        ),
        ("G685Rounding", implements_default!(G685Rounding)),
        (
            "ZustandszahlParams",
            implements_default!(ZustandszahlParams),
        ),
        ("ReactiveLimit", implements_default!(ReactiveLimit)),
        ("En50160Limits", implements_default!(En50160Limits)),
    ]
    .into_iter()
    .filter_map(|(name, has)| has.then_some(name))
    .collect();
    assert!(
        found.is_empty(),
        "these parameter types gained a Default, which answers for the caller: {found:?}"
    );
}

/// The probe tells the two cases apart — checked when the test compiles.
#[test]
fn the_probe_sees_a_default() {
    #[derive(Default)]
    struct Defaulted;
    struct Plain;
    const { assert!(implements_default!(Defaulted)) };
    const { assert!(!implements_default!(Plain)) };
}

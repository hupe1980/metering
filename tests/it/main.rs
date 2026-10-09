//! The integration tests, as one binary.
//!
//! One module per area of the crate. `published_examples` holds only the
//! tests that reproduce a number printed in a source document; `scanners`
//! holds the tests that read the repository's own files and hold a convention
//! there.

mod allocation;
mod berechnungsformel;
mod billing;
mod calendar;
mod eeg;
mod order_independence;
mod published_examples;
mod quantity_invariants;
mod scale;
mod scanners;
#[cfg(feature = "serde")]
mod serde_representation;
mod string_canonicalisation;
mod validation;

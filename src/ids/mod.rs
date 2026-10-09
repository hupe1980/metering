//! Market identifiers: MaLo/MeLo-IDs, EIC, BDEW codes, Regelzonen and OBIS
//! codes.

pub(crate) mod codes;
#[expect(
    clippy::module_inception,
    reason = "the file is private; its items are re-exported at `ids::`"
)]
mod ids;
pub mod obis;

pub use ids::*;

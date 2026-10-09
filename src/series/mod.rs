//! The metered time series: [`MeterInterval`](interval::MeterInterval), the
//! validated [`Series`], register readings and resampling.

pub mod interval;
pub mod reading;
pub mod resample;
#[expect(
    clippy::module_inception,
    reason = "the file is private; its items are re-exported at `series::`"
)]
mod series;

pub use series::*;

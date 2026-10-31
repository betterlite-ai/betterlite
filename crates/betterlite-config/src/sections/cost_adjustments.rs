#[cfg(not(clippy))]
pub use crate::cost_adjustments::{Raw, Settings};

#[cfg(clippy)]
use crate::cost_adjustments::{Raw as CostRaw, Settings as CostSettings};
#[cfg(clippy)]
pub type Raw = CostRaw;
#[cfg(clippy)]
pub type Settings = CostSettings;

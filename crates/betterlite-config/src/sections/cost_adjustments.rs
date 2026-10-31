#![expect(
    clippy::pub_use,
    reason = "sections re-export their Raw/Settings from the crate module that owns them"
)]

pub use crate::cost_adjustments::{Raw, Settings};

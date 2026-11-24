#![expect(
    clippy::disallowed_types,
    reason = "the bridge carries the caller's raw chat kwargs and B919's hidden params, whose values are provider JSON with no closed schema"
)]

pub mod collect;
pub mod error;
pub mod handler;
pub mod input;
pub mod port;
pub mod stream;

mod tests;

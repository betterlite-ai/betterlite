#![expect(
    clippy::disallowed_types,
    reason = "provider JSON carries no closed schema here"
)]

pub mod collect;
pub mod error;
pub mod handler;
pub mod input;
pub mod port;
pub mod stream;

mod tests;

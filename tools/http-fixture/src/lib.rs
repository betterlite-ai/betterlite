#![expect(
    clippy::disallowed_methods,
    reason = "the fixture's server thread runs outside any async runtime, so the clippy.toml reason does not apply: it sleeps out the pause, and one test waits past the old accept deadline"
)]

pub mod error;
pub mod reply;
pub mod request;
pub mod server;

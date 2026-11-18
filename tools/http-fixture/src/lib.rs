#![expect(
    clippy::disallowed_methods,
    reason = "the fixture runs on plain threads with no async runtime: the server thread sleeps out its pause and one test waits past the old accept deadline"
)]

pub mod error;
pub mod reply;
pub mod request;
pub mod server;

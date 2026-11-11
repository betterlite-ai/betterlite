pub mod display;
pub mod error;
pub mod polling;
pub mod selection;
pub mod session;
pub mod teams;
pub mod terminal;
pub mod transport;
pub mod wire;

mod tests;

pub const POLL_SECRET_HEADER: &str = "x-litellm-cli-poll-secret";
pub const SSO_START_PATH: &str = "/sso/cli/start";
pub const SSO_POLL_PATH: &str = "/sso/cli/poll/";
pub const SSO_START_TIMEOUT_SECONDS: u64 = 10;
pub const DEFAULT_TOTAL_TIMEOUT_SECONDS: u64 = 300;
pub const DEFAULT_POLL_INTERVAL_SECONDS: u64 = 2;
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_LOG_EVERY: u32 = 10;
pub const RATE_LIMITED_STATUS: u16 = 429;
pub const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";

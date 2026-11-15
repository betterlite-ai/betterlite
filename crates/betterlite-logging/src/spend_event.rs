pub mod build;
pub mod decode;
pub mod error;
pub mod event;
pub mod input;
pub mod presence;
pub mod reservation;
pub mod response;
pub mod restore;
pub mod time;
pub mod tools;
pub mod usage;

mod tests;

pub const SPEND_EVENT_VERSION: u8 = 1;
pub const CACHE_OFF_KEY: &str = "Cache OFF";
// A collector line holds one request's compact projection, so a line past this size is not one
pub const MAX_EVENT_BYTES: usize = 64 * 1024 * 1024;

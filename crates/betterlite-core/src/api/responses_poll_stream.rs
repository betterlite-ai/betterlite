pub mod accumulator;
pub mod clock;
pub mod event;
pub mod frame;
pub mod ports;
pub mod worker;

mod tests;

pub const ROUTE_TYPE: &str = "aresponses";
// background_streaming.py's UPDATE_INTERVAL, in the milliseconds the clock reports
pub const UPDATE_INTERVAL: u64 = 150;
pub const ERROR_TYPE: &str = "internal_error";
pub const ERROR_CODE: &str = "background_streaming_error";

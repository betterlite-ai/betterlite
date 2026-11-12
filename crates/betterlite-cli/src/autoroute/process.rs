pub const DEFAULT_AUTOROUTE_PORT: u16 = 5483;

pub mod control;
pub mod error;
pub mod launcher;
pub mod liveliness;
pub mod log_tail;
pub mod pid_record;
pub mod port;
pub mod runtime;

mod tests;

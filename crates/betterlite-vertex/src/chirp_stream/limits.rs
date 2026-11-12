pub const CLOSE_REASON_MAX_CHARS: usize = 120;
pub const OUTBOX_SIZE: usize = 256;
pub const REQUEST_QUEUE_SIZE: usize = 64;
// LiteLLM closes a failed realtime session with code 1011
pub const STREAM_FAILURE_CLOSE_CODE: u16 = 1011;
pub const STREAM_DEADLINE_SECONDS: f64 = 280.0;
pub const STREAM_ROTATION_SECONDS: f64 = 240.0;

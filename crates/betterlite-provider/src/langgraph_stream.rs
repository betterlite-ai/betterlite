pub mod error;
pub mod iterator;
pub mod stream;
pub mod wrapper;

mod tests;

use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use futures_util::stream::BoxStream;
use uuid::Uuid;

pub const CHUNK_OBJECT: &str = "chat.completion.chunk";
pub const CONTENT_ROLE: &str = "assistant";
pub const DATA_PREFIX: &str = "data:";
pub const FINISH_STOP: &str = "stop";
pub const FIRST_STREAM_RESPONSE: &str = "first stream response received";
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
pub const MAX_BODY_BYTES: usize = 0x0100_0000;
pub const MAX_LINE_BYTES: usize = TransportLimits::DEFAULT.body_bytes;
pub const MAX_LOGGED_LINE: usize = 100;
pub const OK_STATUS: u16 = 200;
pub const STREAMING_TIMEOUT: Duration = Duration::from_secs(600);

const ERROR_LOG: &str = "Error in LangGraph SSE stream";

pub type Chunks = BoxStream<'static, Result<Vec<u8>, HttpError>>;

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

pub struct UuidV4;

impl IdSource for UuidV4 {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

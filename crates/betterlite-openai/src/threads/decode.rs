use serde::de::DeserializeOwned;
use serde_json::error::Category;

use crate::threads::error::{ReplyFault, ThreadsError};

pub fn decode_thread_message_reply<T>(bytes: &[u8]) -> Result<T, ThreadsError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|source| {
        let fault = match source.classify() {
            Category::Eof => ReplyFault::Truncated,
            Category::Data => ReplyFault::Shape,
            Category::Syntax | Category::Io => ReplyFault::Malformed,
        };
        ThreadsError::Reply { fault, source }
    })
}

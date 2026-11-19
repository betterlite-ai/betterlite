use std::sync::Arc;

use axum::http::request::Parts;
use betterlite_models::model_registration::JsonObject;

use super::ports::BufferedOutput;
use crate::sse_data_generator::types::{ItemStream, StreamSnapshot};

pub struct SharedSource {
    pub items: ItemStream,
    pub snapshot: Option<Arc<dyn StreamSnapshot>>,
    pub buffered: Option<Arc<dyn BufferedOutput>>,
}

pub struct SharedCall<Key> {
    pub source: SharedSource,
    pub key: Key,
    pub request_data: JsonObject,
    pub request: Option<Parts>,
}

// The HTTP failure the generator re-raises; the route ends the body with it, since the status line is already sent
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAbort {
    pub status: u16,
    pub detail: String,
}

use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::bedrock::model_routes::ParamMap;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) params: ParamMap,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) bearer: Option<String>,
    pub(crate) asynchronous: bool,
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) frames: Vec<Vec<u8>>,
    pub(crate) consume: bool,
    #[serde(skip)]
    pub(crate) native: Native,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) url: String,
    pub(crate) body: String,
    pub(crate) signed_body: Vec<u8>,
    pub(crate) response_headers: BTreeMap<String, String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) status: Option<u16>,
    pub(crate) content: Option<String>,
    pub(crate) stream: bool,
    pub(crate) wrapper_model: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) chunks: usize,
    pub(crate) reads: usize,
    pub(crate) sends: usize,
    pub(crate) drops: usize,
    pub(crate) credentials: usize,
    pub(crate) factory: usize,
    pub(crate) factory_timeout_ms: Option<u128>,
    pub(crate) chunk_size: Option<u64>,
    pub(crate) logs: Vec<Json>,
    pub(crate) error: Option<String>,
    pub(crate) mapped_error: Option<ErrorClass>,
    pub(crate) reasoning_tokens: Option<i64>,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub(crate) enum Failure {
    #[default]
    None,
    Timeout,
    Closed,
}

#[derive(Clone, Default)]
pub(crate) struct Native {
    pub(crate) failure: Failure,
    pub(crate) supplied: Option<ClientKind>,
    pub(crate) skip_log: bool,
    pub(crate) chunk_size: Option<u64>,
}

use std::str::from_utf8;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};
use error::FileRoutingIdsError;

pub mod batch;
pub mod error;
pub mod legacy;
pub mod routing;
mod tests;

pub const MAX_FILE_LIST_LIMIT: i64 = 10_000;
pub const FILE_LIST_CHUNK_SIZE: usize = 500;
pub const BATCH_CREATE_HIDDEN_PARAM: &str = "batch_create";
pub const EXECUTED_BATCH_ID_PREFIX: &str = "litellm_batch_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingIdType {
    File,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRoutingIds {
    pub unified_id: Option<String>,
    pub model: Option<String>,
    pub original_id: String,
}

#[async_trait]
pub trait ManagedResourceAccessChecker: Send + Sync {
    async fn can_call_unified_file_id(&self, unified_file_id: &str, principal: &Principal) -> bool;
    async fn can_call_unified_object_id(
        &self,
        unified_object_id: &str,
        principal: &Principal,
    ) -> bool;
}

pub fn decode_file_routing_ids(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<FileRoutingIds, FileRoutingIdsError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    let text = from_utf8(bytes).map_err(|source| FileRoutingIdsError::NotUtf8 { source })?;
    Ok(FileRoutingIds {
        unified_id: legacy::decode_legacy_unified_id(text),
        model: routing::decode_model_from_file_id(text),
        original_id: routing::get_original_file_id(text),
    })
}

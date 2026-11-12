use std::future::Future;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::anthropic::{AnthropicCompaction, ModelInfoLookup};
use super::error::NativeCompactionError;

pub type Mapping = IndexMap<String, Json>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CompactionProtocol {
    Chat,
    Messages,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompactionParams<'params> {
    pub model: &'params str,
    pub custom_llm_provider: Option<&'params str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactionKind {
    Summarize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Compaction {
    #[serde(rename = "type")]
    pub kind: CompactionKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ExtraHeaders {
    #[serde(rename = "anthropic-beta")]
    pub anthropic_beta: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RequestKwargs {
    pub compaction: Compaction,
    pub extra_headers: ExtraHeaders,
}

pub trait NativeCompactionProvider: Sync {
    fn supports_native_compaction(
        &self,
        params: &CompactionParams<'_>,
    ) -> impl Future<Output = bool> + Send;

    fn compatible_defaults(&self, payload: &Mapping) -> bool;

    fn request_kwargs(&self) -> RequestKwargs;

    fn extract_summary(
        &self,
        protocol: CompactionProtocol,
        response: &Mapping,
    ) -> Result<Option<String>, NativeCompactionError>;
}

pub async fn get_native_compaction_provider<'lookup, Lookup>(
    lookup: &'lookup Lookup,
    params: &CompactionParams<'_>,
) -> Option<AnthropicCompaction<'lookup, Lookup>>
where
    Lookup: ModelInfoLookup,
{
    let provider = AnthropicCompaction { lookup };
    provider
        .supports_native_compaction(params)
        .await
        .then_some(provider)
}

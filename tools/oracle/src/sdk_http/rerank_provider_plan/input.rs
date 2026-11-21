use super::super::error::SdkHttpError;
use super::{Call, MOCK};
use betterlite_core::api::rerank_provider_plan::call::{RerankCall, ReturnDocuments};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
// A `{mock}` value names the harness origin in settings, env and kwargs alike, so kwargs substitute through their JSON text
pub(super) fn core_call(call: &Call, api_base: &str) -> Result<RerankCall, SdkHttpError> {
    let text = serde_json::to_string(&call.kwargs)
        .map_err(SdkHttpError::Serialize)?
        .replace(MOCK, api_base);
    let kwargs: IndexMap<String, Json> =
        serde_json::from_str(&text).map_err(SdkHttpError::Serialize)?;
    Ok(RerankCall {
        model: call.model.clone(),
        query: call.query.clone(),
        documents: call.documents.clone(),
        custom_llm_provider: call.custom_llm_provider.clone(),
        top_n: call.top_n,
        rank_fields: call.rank_fields.clone(),
        return_documents: match call.return_documents {
            None => ReturnDocuments::Default,
            Some(None) => ReturnDocuments::Null,
            Some(Some(value)) => ReturnDocuments::Value(value),
        },
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        kwargs,
    })
}
pub(super) fn settings_document(call: &Call, api_base: &str) -> Result<String, SdkHttpError> {
    let settings =
        serde_json::to_string(&call.litellm_settings).map_err(SdkHttpError::Serialize)?;
    Ok(format!("{{\"litellm_settings\":{settings}}}").replace(MOCK, api_base))
}

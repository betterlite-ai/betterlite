use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use jiff::Timestamp;
use serde::Serialize;

use crate::cache_prefix::ttl::extract_cached_ttl;
use crate::gemini_history::content::Content;
use crate::gemini_history::convert::convert_messages_with_history;
use crate::gemini_history::error::HistoryError;
use crate::model_policy::capabilities::{
    ModelCapabilityLookup, VertexCapabilityProvider, get_supports_system_message,
};
use crate::system_instruction::{SystemInstruction, extract_system_instruction};

pub struct CachedContentRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub provider: VertexCapabilityProvider,
    pub cache_key: &'request str,
    pub project: &'request str,
    pub location: &'request str,
    pub disable_transform: bool,
    pub now: Timestamp,
}

#[derive(Debug, Serialize)]
pub struct CachedContentRequestBody {
    pub contents: Vec<Content>,
    pub model: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<SystemInstruction>,
}

pub async fn cached_content_request<Source>(
    request: &CachedContentRequest<'_>,
    lookup: &(dyn ModelCapabilityLookup + Sync),
    fetcher: &Source,
) -> Result<CachedContentRequestBody, HistoryError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let ttl = extract_cached_ttl(request.messages, request.disable_transform, request.now);
    let supports_system_message =
        get_supports_system_message(request.model, request.provider, lookup);
    let extraction = extract_system_instruction(supports_system_message, request.messages);
    let history =
        convert_messages_with_history(&extraction.messages, Some(request.model), fetcher).await?;
    let model = match request.provider {
        VertexCapabilityProvider::VertexAi | VertexCapabilityProvider::VertexAiBeta => format!(
            "projects/{}/locations/{}/publishers/google/models/{}",
            request.project, request.location, request.model
        ),
        VertexCapabilityProvider::Gemini => format!("models/{}", request.model),
    };
    Ok(CachedContentRequestBody {
        contents: history.contents,
        model,
        display_name: request.cache_key.to_owned(),
        ttl,
        system_instruction: extraction.instruction,
    })
}

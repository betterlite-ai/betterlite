use std::collections::BTreeMap;

use betterlite_core::api::codex_history_tools::normalize_codex_input_items;
use betterlite_logging::analytics::python::repr;
use betterlite_openai::response_input::{
    ResponsesInput, transform_responses_request as openai_request,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media::limits::MAX_REMOTE_URLS;

use crate::model_routes::CHAT_COMPLETIONS_PREFIX;
use crate::responses::config::CUSTOM_LLM_PROVIDER;
use crate::responses::error::ResponsesError;
use crate::responses::images::{collect_remote_image_urls, inline_remote_image_urls};

// LiteLLM has no ceiling here, so a request could make the gateway download without end
fn remote_urls_within_limit(input: &ResponsesInput) -> Result<Vec<String>, ResponsesError> {
    let urls = collect_remote_image_urls(input);
    if urls.len() > MAX_REMOTE_URLS {
        return Err(ResponsesError::Media(MediaError::TooManyRemoteUrls {
            limit: MAX_REMOTE_URLS,
        }));
    }
    Ok(urls)
}

#[derive(Debug, Clone, Copy)]
pub enum FetchMode {
    Each,
    Batch,
}

#[derive(Debug, Clone, Copy)]
pub struct TransformRequest<'request> {
    pub model: &'request str,
    pub input: &'request ResponsesInput,
    pub params: &'request Object,
    pub model_info: Option<&'request Object>,
}

// Each is LiteLLM's sync path, one convert_url per distinct URL in first-seen order; Batch is its async path, one convert_urls call
pub async fn transform_responses<Source>(
    fetcher: &Source,
    mode: FetchMode,
    request: &TransformRequest<'_>,
) -> Result<Object, ResponsesError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let urls = remote_urls_within_limit(request.input)?;
    let mut inlined = BTreeMap::new();
    match mode {
        FetchMode::Each => {
            for url in urls {
                let data_url = fetcher.convert_url(&url).await?;
                inlined.insert(url, data_url);
            }
        }
        FetchMode::Batch => {
            if !urls.is_empty() {
                let data_urls = fetcher.convert_urls(&urls).await?;
                inlined.extend(urls.into_iter().zip(data_urls));
            }
        }
    }
    let inlined_input = inline_remote_image_urls(request.input, &inlined)?;
    transform_inlined_request(
        request.model,
        &inlined_input,
        request.params,
        request.model_info,
    )
}

pub fn transform_inlined_request(
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, ResponsesError> {
    let raw_input = serde_json::to_value(input).map_err(ResponsesError::Encode)?;
    let (normalized, rewritten) = normalize_codex_input_items(&raw_input);
    if !rewritten.is_empty() {
        let quoted = rewritten
            .iter()
            .map(repr::<String>)
            .collect::<Result<Vec<_>, _>>()?;
        let comma = if rewritten.len() == 1 { "," } else { "" };
        tracing::warn!(
            "Bedrock Runtime Responses API: rewrote Codex input item type(s) ({}{comma}) that the endpoint rejects.",
            quoted.join(", ")
        );
    }
    let request_input: ResponsesInput =
        serde_json::from_value(normalized).map_err(ResponsesError::Encode)?;
    let routed = model.strip_prefix(CHAT_COMPLETIONS_PREFIX).unwrap_or(model);
    Ok(openai_request(
        routed,
        &request_input,
        params,
        CUSTOM_LLM_PROVIDER,
        model_info,
    )?)
}

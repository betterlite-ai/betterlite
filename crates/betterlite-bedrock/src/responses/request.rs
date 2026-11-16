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

// One convert_url per distinct URL in first-seen order, all before anything reaches the transport
pub async fn transform_responses_request<Source>(
    fetcher: &Source,
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, ResponsesError>
where
    Source: RemoteMediaFetcher,
{
    let mut inlined = BTreeMap::new();
    for url in remote_urls_within_limit(input)? {
        let data_url = fetcher.convert_url(&url).await?;
        inlined.insert(url, data_url);
    }
    let inlined_input = inline_remote_image_urls(input, &inlined)?;
    transform_inlined_request(model, &inlined_input, params, model_info)
}

// One convert_urls call carries every distinct URL and answers in input order
pub async fn async_transform_responses<Source>(
    fetcher: &Source,
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, ResponsesError>
where
    Source: RemoteMediaFetcher,
{
    let urls = remote_urls_within_limit(input)?;
    let data_urls = if urls.is_empty() {
        Vec::new()
    } else {
        fetcher.convert_urls(&urls).await?
    };
    let inlined: BTreeMap<String, String> = urls.into_iter().zip(data_urls).collect();
    let inlined_input = inline_remote_image_urls(input, &inlined)?;
    transform_inlined_request(model, &inlined_input, params, model_info)
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

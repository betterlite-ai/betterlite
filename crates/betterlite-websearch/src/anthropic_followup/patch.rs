use betterlite_context::agentic_loop_plan::{
    AgenticLoopRequestPatch, FollowupMessage, JsonMap, get_text,
};
use serde::Deserialize;
use serde_json::value::to_raw_value;

use super::AnthropicFollowupError;
use super::followup_kwargs::prepare_followup_kwargs;
use super::max_tokens::resolve_max_tokens;
use crate::call_transformation::WebSearchTransformation;
use crate::call_transformation::outcome::search_outcome_text;
use crate::call_transformation::wire::{Call, Results};
use crate::chat_responses_followup::search::{SearchExecutor, SearchSurface, search_batch};
use crate::native_blocks::{ResponseJson, inject_native_blocks};
use crate::records::SearchOutcome;

const ANTHROPIC_FORMAT: &str = "anthropic";
const MAX_TOKENS_KEY: &str = "max_tokens";
const CALL_ID_KEY: &str = "litellm_call_id";
const AGENTIC_LOOP_KEY: &str = "agentic_loop_params";

pub struct FollowupLogging<'logging> {
    pub call_id: Option<&'logging str>,
    pub model_call_details: &'logging JsonMap,
}

pub struct AnthropicPatchRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [FollowupMessage],
    pub tool_calls: &'request [Call],
    pub thinking_blocks: &'request [ResponseJson],
    pub optional_params: &'request JsonMap,
    pub logging: Option<&'request FollowupLogging<'request>>,
    pub kwargs: &'request JsonMap,
}

#[derive(Deserialize)]
struct AgenticLoopParams {
    #[serde(default)]
    model: Option<String>,
}

pub async fn build_anthropic_request_patch(
    executor: &dyn SearchExecutor,
    request: &AnthropicPatchRequest<'_>,
) -> Result<(AgenticLoopRequestPatch, Vec<SearchOutcome>), AnthropicFollowupError> {
    let outcomes = search_batch(
        executor,
        request.tool_calls,
        request.kwargs,
        SearchSurface::Chat,
    )
    .await;
    let messages = followup_messages(request, &outcomes)?;
    let max_tokens = resolve_max_tokens(request.optional_params, request.kwargs)?;
    tracing::debug!("WebSearchInterception: Using max_tokens={max_tokens} for follow-up request");
    let model = patch_model(request.logging, request.model)?;
    tracing::debug!(
        "{}",
        patch_log_line(request, &model, messages.len(), outcomes.len())
    );
    let patch = AgenticLoopRequestPatch {
        model: Some(model),
        messages: Some(messages),
        tools: None,
        max_tokens: Some(max_tokens),
        optional_params: strip_max_tokens(request.optional_params),
        kwargs: prepare_followup_kwargs(request.kwargs),
    };
    Ok((patch, outcomes))
}

fn followup_messages(
    request: &AnthropicPatchRequest<'_>,
    outcomes: &[SearchOutcome],
) -> Result<Vec<FollowupMessage>, AnthropicFollowupError> {
    let texts: Vec<String> = outcomes.iter().map(search_outcome_text).collect();
    let (assistant, results) =
        WebSearchTransformation::transform_response(request.tool_calls, &texts, ANTHROPIC_FORMAT)?;
    let message = inject_native_blocks(serde_json::to_value(assistant)?, request.thinking_blocks);
    let mut messages: Vec<FollowupMessage> = request.messages.to_vec();
    messages.push(FollowupMessage(to_raw_value(&message)?));
    match results {
        Results::Anthropic(user) => messages.push(FollowupMessage(to_raw_value(&user)?)),
        Results::OpenAi(_) => return Err(AnthropicFollowupError::UnexpectedResultFormat),
    }
    Ok(messages)
}

fn strip_max_tokens(optional_params: &JsonMap) -> JsonMap {
    optional_params
        .iter()
        .filter(|(key, _value)| key.as_str() != MAX_TOKENS_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn patch_model(
    logging: Option<&FollowupLogging<'_>>,
    model: &str,
) -> Result<String, AnthropicFollowupError> {
    let Some(details) = logging.map(|logging| logging.model_call_details) else {
        return Ok(model.to_owned());
    };
    let Some(raw) = details.get(AGENTIC_LOOP_KEY) else {
        return Ok(model.to_owned());
    };
    let params: AgenticLoopParams = serde_json::from_str(raw.get())
        .map_err(AnthropicFollowupError::InvalidAgenticLoopParams)?;
    Ok(params.model.unwrap_or_else(|| model.to_owned()))
}

fn call_id(logging: Option<&FollowupLogging<'_>>, kwargs: &JsonMap) -> String {
    let logged = logging
        .and_then(|logging| logging.call_id)
        .filter(|id| !id.is_empty());
    logged.map_or_else(
        || {
            get_text(kwargs, CALL_ID_KEY)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| "unknown".to_owned())
        },
        str::to_owned,
    )
}

pub(super) fn patch_log_line(
    request: &AnthropicPatchRequest<'_>,
    model: &str,
    messages: usize,
    searches: usize,
) -> String {
    let call = call_id(request.logging, request.kwargs);
    format!(
        "WebSearchInterception: Built anthropic request patch [call_id={call} model={model} messages={messages} searches={searches}]"
    )
}

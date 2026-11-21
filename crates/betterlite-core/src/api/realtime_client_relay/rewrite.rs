use std::error::Error;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrails::runner::{RealtimeGuardrails, RunRealtimeGuardrails as _};

use super::RealtimeClientRelay;
use super::error::RealtimeClientRelayError;

const ITEM_CREATE: &str = "conversation.item.create";
const FUNCTION_CALL_OUTPUT: &str = "function_call_output";
const INPUT_TEXT: &str = "input_text";
const USER_ROLE: &str = "user";

// The sanitized tool result LiteLLM hands the backend before the guardrail's own frames; compact JSON, not json.dumps spacing
pub(crate) const TOOL_BLOCK_MARKER: &str = r#"{"error":"Tool output blocked by content policy"}"#;

// LiteLLM parses the frame once and reads its type; a frame that is not a JSON object skips every rewrite
pub(crate) struct ClientEvent {
    pub(crate) value: Json,
    pub(crate) message: String,
    pub(crate) msg_type: Option<String>,
}

impl ClientEvent {
    pub(crate) fn parse(message: &str) -> Option<Self> {
        let Ok(value) = serde_json::from_str::<Json>(message) else {
            return None;
        };
        let msg_type = type_name(&value).map(str::to_owned);
        if !matches!(value, Json::Object(_)) {
            return None;
        }
        Some(Self {
            value,
            message: message.to_owned(),
            msg_type,
        })
    }

    pub(crate) fn refresh(&mut self) -> Result<(), serde_json::Error> {
        self.message = serde_json::to_string(&self.value)?;
        Ok(())
    }
}

fn type_name(value: &Json) -> Option<&str> {
    let Json::Object(fields) = value else {
        return None;
    };
    match fields.get("type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// LiteLLM checks a function_call_output before role=user, so a tool result cannot skip the policy as user text
pub(crate) async fn client_guardrails<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    event: &ClientEvent,
) -> Result<bool, RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    if event.msg_type.as_deref() != Some(ITEM_CREATE) {
        return Ok(false);
    }
    let Json::Object(root) = &event.value else {
        return Ok(false);
    };
    let Some(Json::Object(item)) = root.get("item") else {
        return Ok(false);
    };
    if matches!(item.get("type"), Some(Json::Text(kind)) if kind == FUNCTION_CALL_OUTPUT) {
        return tool_guardrails(relay, root, item).await;
    }
    if !matches!(item.get("role"), Some(Json::Text(role)) if role == USER_ROLE) {
        return Ok(false);
    }
    let Some(combined) =
        combined_text(item.get("content")).map_err(|()| RealtimeClientRelayError::GuardrailText)?
    else {
        return Ok(false);
    };
    if combined.is_empty() {
        return Ok(false);
    }
    let blocked = run_guardrails(relay, &combined, None).await?;
    if blocked {
        relay.state.setup.pending_guardrail_message = Some(combined);
    }
    Ok(blocked)
}

// A function_call_output is checked on its own, and its sanitized message goes with the guardrail's pre-call frames
async fn tool_guardrails<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    root: &IndexMap<String, Json>,
    item: &IndexMap<String, Json>,
) -> Result<bool, RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let Some(text) = output_text(item)? else {
        return Ok(false);
    };
    let sanitized = sanitized_tool_message(root, item)?;
    run_guardrails(relay, &text, Some(&sanitized)).await
}

// LiteLLM stringifies a non-string tool output with json.dumps; an empty one is never checked
fn output_text(item: &IndexMap<String, Json>) -> Result<Option<String>, serde_json::Error> {
    match item.get("output") {
        None => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(
            value
            @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)),
        ) => serde_json::to_string(value).map(Some),
    }
}

pub(crate) fn sanitized_tool_message(
    root: &IndexMap<String, Json>,
    item: &IndexMap<String, Json>,
) -> Result<String, serde_json::Error> {
    let mut sanitized_item = item.clone();
    sanitized_item.insert(
        "output".to_owned(),
        Json::Text(TOOL_BLOCK_MARKER.to_owned()),
    );
    let mut message = root.clone();
    message.insert("item".to_owned(), Json::Object(sanitized_item));
    serde_json::to_string(&Json::Object(message))
}

// LiteLLM joins the input_text blocks with spaces; a content that is not iterable of blocks either raises or yields none
fn combined_text(content: Option<&Json>) -> Result<Option<String>, ()> {
    let items = match content {
        None | Some(Json::Text(_) | Json::Object(_)) => return Ok(None),
        Some(Json::List(items)) => items,
        Some(Json::Null | Json::Bool(_) | Json::Number(_)) => return Err(()),
    };
    let mut texts = Vec::new();
    for entry in items {
        let Json::Object(fields) = entry else {
            continue;
        };
        if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == INPUT_TEXT) {
            continue;
        }
        match fields.get("text") {
            None => texts.push(String::new()),
            Some(Json::Text(text)) => texts.push(text.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => {
                return Err(());
            }
        }
    }
    Ok(Some(texts.join(" ")))
}

async fn run_guardrails<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    transcript: &str,
    pre_block: Option<&str>,
) -> Result<bool, RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let mut runner = RealtimeGuardrails {
        state: &mut *relay.state,
        provider: relay.provider,
        guardrails: relay.guardrails,
    };
    runner
        .run_realtime_guardrails(
            transcript,
            None,
            pre_block,
            Some(&[GuardrailEventHooks::PreCall]),
        )
        .await
        .map_err(RealtimeClientRelayError::Guardrails)
}

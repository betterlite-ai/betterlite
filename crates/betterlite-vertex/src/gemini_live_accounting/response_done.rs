use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::Number;
use uuid::Uuid;

use super::aliases::add_usage_detail_aliases;
use super::error::LiveAccountingError;
use super::usage::{chat_usage, consume_usage_metadata};
use crate::gemini_live_client::records::ServerMessage;
use crate::gemini_live_client::state::LiveSessionState;
use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::session::MaxResponseTokens;
use crate::gemini_live_setup::wire::GenerationConfig;

#[derive(Clone, Copy, Debug)]
pub struct ResponseDoneInput<'input> {
    pub response_id: Option<&'input str>,
    pub conversation_id: Option<&'input str>,
    pub output_items: &'input [Object],
    pub session_configuration_request: Option<&'input str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResponseDoneEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response: ResponseDoneObject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResponseDoneObject {
    pub object: &'static str,
    pub id: String,
    pub status: &'static str,
    pub status_details: Option<Object>,
    pub output: Vec<Object>,
    pub conversation_id: String,
    pub modalities: Vec<String>,
    pub usage: Object,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<MaxResponseTokens>,
}

// The chat-to-Responses usage projection comes in as a function so this leaf never imports core
pub fn transform_response_done_event(
    state: &mut LiveSessionState,
    frame: &mut ServerMessage,
    input: &ResponseDoneInput<'_>,
    project_usage: &dyn Fn(&Usage) -> ResponseApiUsage,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<ResponseDoneEvent, LiveAccountingError> {
    let conversation_id = input
        .conversation_id
        .map_or_else(|| format!("conv_{}", uuids()), str::to_owned);
    let response_id = input
        .response_id
        .map_or_else(|| format!("resp_{}", uuids()), str::to_owned);
    let setup = generation_config(input.session_configuration_request)?;
    let modalities = setup
        .response_modalities
        .unwrap_or_else(|| vec!["AUDIO".to_owned()])
        .iter()
        .map(|modality| modality.to_lowercase())
        .collect();
    let reported = consume_usage_metadata(state, frame);
    let grounding = frame
        .server_content
        .as_ref()
        .and_then(|content| content.grounding_metadata.as_ref());
    let chat = chat_usage(reported.as_ref(), grounding)?;
    let mut usage: Object = serde_json::from_slice(&serde_json::to_vec(&project_usage(&chat))?)?;
    add_usage_detail_aliases(&mut usage);
    Ok(ResponseDoneEvent {
        event_type: "response.done",
        event_id: format!("event_{}", uuids()),
        response: ResponseDoneObject {
            object: "realtime.response",
            id: response_id,
            status: "completed",
            status_details: None,
            output: input.output_items.to_vec(),
            conversation_id,
            modalities,
            usage,
            temperature: setup.temperature,
            max_output_tokens: setup.max_output_tokens,
        },
    })
}

fn generation_config(stored_setup: Option<&str>) -> Result<GenerationConfig, LiveAccountingError> {
    match stored_setup.filter(|text| !text.is_empty()) {
        Some(text) => Ok(decode_gemini_live_setup(text)?.generation_config),
        None => Ok(GenerationConfig::default()),
    }
}

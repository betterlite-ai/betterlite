#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as WireJson;
use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::responses_objects::{
    InputTokensDetails, OutputTokensDetails, ResponseApiUsage,
};
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;
use uuid::Uuid;

use crate::gemini_live_client::records::ServerMessage;
use crate::gemini_live_client::state::{LiveSessionState, ToolCallNames};

pub(crate) const TRANSCRIBE_MODEL: &str = "gemini/gemini-3.5-transcribe-live";
pub(crate) const LIVE_MODEL: &str = "gemini-2.5-flash";

const CATALOG: &str = r#"{
    "gemini-2.5-flash": {"litellm_provider": "gemini", "mode": "realtime"},
    "gemini/gemini-3.5-transcribe-live": {"litellm_provider": "gemini", "mode": "audio_transcription"}
}"#;

// every identifier repeats one byte, so the first sixteen hex digits of a call id show which draw made it
pub(crate) fn uuids() -> impl FnMut() -> Uuid {
    let mut drawn = 0_u128;
    move || {
        drawn = drawn.saturating_add(1);
        Uuid::from_u128(drawn.saturating_mul(0x0101_0101_0101_0101_0101_0101_0101_0101))
    }
}

pub(crate) fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the test catalog parses")
}

pub(crate) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the object parses")
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the JSON parses")
}

pub(crate) fn state_with_pending(source: &str) -> LiveSessionState {
    LiveSessionState {
        tool_call_names: ToolCallNames::default(),
        pending_usage_metadata: Some(object(source)),
        unbilled_input_audio_bytes: 0,
    }
}

pub(crate) fn frame(source: &str) -> ServerMessage {
    serde_json::from_str(source).expect("the frame parses")
}

pub(crate) fn field<'object>(object: &'object Object, name: &str) -> &'object Json {
    object.get(name).expect("the field is present")
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

// a stand-in for the Responses projection the composition root injects, covering the fields these tests read
pub(crate) fn project(usage: &Usage) -> ResponseApiUsage {
    let grounding: BTreeMap<String, WireJson> = usage
        .prompt_tokens_details
        .iter()
        .filter_map(|details| details.web_search_requests)
        .map(|count| {
            (
                "web_search_requests".to_owned(),
                WireJson::Number(Number::from(count)),
            )
        })
        .collect();
    ResponseApiUsage {
        input_tokens: usage.prompt_tokens,
        input_tokens_details: usage.prompt_tokens_details.as_ref().map(|details| {
            InputTokensDetails {
                audio_tokens: details.audio_tokens,
                cached_tokens: details.cached_tokens.unwrap_or(0),
                cached_tokens_details: None,
                image_tokens: None,
                text_tokens: details.text_tokens,
                video_tokens: None,
                extra: grounding,
            }
        }),
        output_tokens: usage.completion_tokens,
        output_tokens_details: usage.completion_tokens_details.as_ref().map(|details| {
            OutputTokensDetails {
                audio_tokens: details.audio_tokens,
                reasoning_tokens: details.reasoning_tokens,
                text_tokens: details.text_tokens,
                extra: BTreeMap::new(),
            }
        }),
        total_tokens: usage.total_tokens,
        cost: None,
        extra: BTreeMap::new(),
    }
}

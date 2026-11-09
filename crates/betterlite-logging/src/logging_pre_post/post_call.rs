use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::args::AdditionalArgs;
use super::env::{LogLevel, PrePostEnv};
use super::hooks::{Phase, run_input_callbacks};
use crate::logging_state::LoggingState;
use crate::python_json;

#[derive(Deserialize)]
pub struct PostCall {
    pub original_response: Json,
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub additional_args: AdditionalArgs,
}

// A dict response is stored as its JSON text, the way LiteLLM's json.dumps does it
fn response_text(response: &Json) -> String {
    match response {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python_json::dumps(response)
        }
    }
}

pub fn post_call(state: &mut LoggingState, call: &PostCall, env: &PrePostEnv<'_>) {
    let response = if let Json::Object(fields) = &call.original_response {
        Json::Text(python_json::dumps_object(fields))
    } else {
        call.original_response.clone()
    };
    let details = &mut state.model_call_details;
    details.insert("input".to_owned(), call.input.clone().unwrap_or(Json::Null));
    details.insert("original_response".to_owned(), response.clone());
    details.insert(
        "additional_args".to_owned(),
        call.additional_args.recorded(),
    );
    details.insert(
        "log_event_type".to_owned(),
        Json::Text("post_api_call".to_owned()),
    );
    let level = if state.litellm_request_debug {
        LogLevel::Warning
    } else {
        LogLevel::Debug
    };
    env.emit(
        level,
        &format!("RAW RESPONSE:\n{}\n\n", response_text(&response)),
        IndexMap::new(),
    );
    env.run_logger_fn(&state.model_call_details);
    run_input_callbacks(state, Phase::Post, env);
}

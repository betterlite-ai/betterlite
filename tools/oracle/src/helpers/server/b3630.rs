use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::pass_through_logging::batch_attribution::{
    is_collection_route, request_tags_from_metadata,
};
use betterlite_server::pass_through_logging::response_costing::{
    cost_relevant_speed, resolve_costing_model,
};
use betterlite_server::pass_through_logging::stream_fields::{Chunk, extract_message_start_field};
use serde::Deserialize;
use serde_json::{Value, json};

const INIT: &str = r#"{"model":"","call_type":"pass_through_endpoint","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"b3630-call","function_id":"b3630","litellm_trace_id":"b3630-trace"}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    routes: Vec<RouteCall>,
    #[serde(default)]
    tags: Vec<TagsCall>,
    #[serde(default)]
    models: Vec<ChunksCall>,
    #[serde(default)]
    ids: Vec<ChunksCall>,
    #[serde(default)]
    costing: Vec<CostingCall>,
    #[serde(default)]
    speeds: Vec<SpeedCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteCall {
    url_route: String,
    collection_suffix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagsCall {
    metadata: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunksCall {
    chunks: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CostingCall {
    model: String,
    details: JsonObject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeedCall {
    request_body: Option<JsonObject>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let routes: Vec<Value> = input
            .routes
            .iter()
            .map(|call| {
                json!(is_collection_route(
                    &call.url_route,
                    &call.collection_suffix
                ))
            })
            .collect();
        let tags: Vec<Value> = input
            .tags
            .iter()
            .map(|call| json!(request_tags_from_metadata(&call.metadata)))
            .collect();
        let speeds: Vec<Value> = input
            .speeds
            .iter()
            .map(|call| json!(cost_relevant_speed(call.request_body.as_ref())))
            .collect();
        Ok(json!({
            "routes": routes,
            "tags": tags,
            "models": field_answers(&input.models, "model")?,
            "ids": field_answers(&input.ids, "id")?,
            "costing": costing_answers(&input.costing)?,
            "speeds": speeds,
        }))
    }
}

fn field_answers(calls: &[ChunksCall], field: &str) -> Result<Vec<Value>, Box<HelperError>> {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let found = extract_message_start_field(&chunks_of(call)?, field).or_value_error()?;
        answers.push(json!(found));
    }
    Ok(answers)
}

fn costing_answers(calls: &[CostingCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let call_log = logging_state(&call.details)?;
        answers.push(json!(resolve_costing_model(&call.model, &call_log)));
    }
    Ok(answers)
}

fn chunks_of(call: &ChunksCall) -> Result<Vec<Chunk>, Box<HelperError>> {
    let mut chunks = Vec::with_capacity(call.chunks.len());
    for encoded in &call.chunks {
        chunks.push(Chunk::Bytes(hex::decode(encoded).or_value_error()?));
    }
    Ok(chunks)
}

fn logging_state(details: &JsonObject) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(INIT).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        || Duration::ZERO,
    )
    .or_value_error()?;
    for (name, value) in details {
        state.model_call_details.insert(name.clone(), value.clone());
    }
    Ok(state)
}

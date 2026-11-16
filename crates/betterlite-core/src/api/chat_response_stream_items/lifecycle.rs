use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::ResponseEvent;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::chat_response_stream_tools::fields::object;
use crate::api::chat_response_stream_tools::state::{ChatStreamState, next_sequence};
use crate::api::response_id_codecs::response::{build_response_id, is_encoded_response_id};
use crate::api::responses_codex_tools::tool_choices::responses_tool_choice;

use super::error::StreamItemsError;
use super::sources::EventSources;
use super::support::numbered;

const DEFAULT_TOP_P: f64 = 1.0;

pub fn create_response_created_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let response = lifecycle_response(state, sources)?;
    let sequence_number = next_sequence(state)?;
    Ok(KnownResponseEvent::ResponseCreated(ResponseEvent {
        response,
        extra: numbered(sequence_number),
    }))
}

pub fn create_response_in_progress_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let response = lifecycle_response(state, sources)?;
    let sequence_number = next_sequence(state)?;
    Ok(KnownResponseEvent::ResponseInProgress(ResponseEvent {
        response,
        extra: numbered(sequence_number),
    }))
}

fn lifecycle_response<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<ResponsesApiResponse, StreamItemsError> {
    let raw_id = match state.progress.cached_response_id.clone() {
        Some(response_id) => response_id,
        None => {
            let response_id = format!("resp_{}", (sources.ids)());
            state.progress.cached_response_id = Some(response_id.clone());
            response_id
        }
    };
    let response_id = encoded_response_id(state, &raw_id)?;
    let mut data = response_data(state, sources)?;
    data.insert("id".to_owned(), Json::Text(response_id));
    decode_response(&data)
}

fn encoded_response_id<Context>(
    state: &ChatStreamState<Context>,
    raw_id: &str,
) -> Result<String, StreamItemsError> {
    if is_encoded_response_id(raw_id) {
        return Ok(raw_id.to_owned());
    }
    let model_id = metadata_model_id(state)?;
    Ok(build_response_id(
        state.custom_llm_provider.as_deref(),
        model_id,
        raw_id,
    ))
}

fn metadata_model_id<Context>(
    state: &ChatStreamState<Context>,
) -> Result<Option<&str>, StreamItemsError> {
    match state.litellm_metadata.get("model_info") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => match fields.get("id") {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Text(id)) => Ok(Some(id)),
            Some(_) => Err(StreamItemsError::Metadata {
                field: "litellm_metadata.model_info.id",
            }),
        },
        Some(_) => Err(StreamItemsError::Metadata {
            field: "litellm_metadata.model_info",
        }),
    }
}

fn response_data<Context>(
    state: &ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<IndexMap<String, Json>, StreamItemsError> {
    let request =
        object(&state.responses_api_request).ok_or(StreamItemsError::ResponseNotObject)?;
    let mut data = fixed_response_fields(state, (sources.clock)()?);
    for name in [
        "instructions",
        "temperature",
        "text",
        "truncation",
        "user",
        "metadata",
    ] {
        let value = request.get(name).cloned().unwrap_or(Json::Null);
        data.insert(name.to_owned(), value);
    }
    let tool_choice = request.get("tool_choice").unwrap_or(&Json::Null);
    data.insert("tool_choice".to_owned(), responses_tool_choice(tool_choice));
    data.insert(
        "tools".to_owned(),
        request
            .get("tools")
            .cloned()
            .unwrap_or_else(|| Json::List(Vec::new())),
    );
    let top_p = match request.get("top_p") {
        Some(value) => value.clone(),
        None => Json::Number(
            Number::from_f64(DEFAULT_TOP_P).ok_or(StreamItemsError::DefaultTopPNotFinite)?,
        ),
    };
    data.insert("top_p".to_owned(), top_p);
    Ok(data)
}

fn fixed_response_fields<Context>(
    state: &ChatStreamState<Context>,
    created_at: i64,
) -> IndexMap<String, Json> {
    let mut data = IndexMap::new();
    data.insert("id".to_owned(), Json::Null);
    data.insert("object".to_owned(), Json::Text("response".to_owned()));
    data.insert(
        "created_at".to_owned(),
        Json::Number(Number::from(created_at)),
    );
    data.insert("status".to_owned(), Json::Text("in_progress".to_owned()));
    data.insert("model".to_owned(), Json::Text(state.model.clone()));
    data.insert("output".to_owned(), Json::List(Vec::new()));
    data.insert("parallel_tool_calls".to_owned(), Json::Bool(true));
    data.insert("reasoning".to_owned(), default_reasoning());
    data.insert("store".to_owned(), Json::Bool(true));
    data
}

fn default_reasoning() -> Json {
    Json::Object(IndexMap::from([
        ("effort".to_owned(), Json::Null),
        ("summary".to_owned(), Json::Null),
    ]))
}

fn decode_response(
    data: &IndexMap<String, Json>,
) -> Result<ResponsesApiResponse, StreamItemsError> {
    let encoded = serde_json::to_vec(data)?;
    let mut decoder = serde_json::Deserializer::from_slice(&encoded);
    serde_path_to_error::deserialize(&mut decoder).map_err(StreamItemsError::from)
}

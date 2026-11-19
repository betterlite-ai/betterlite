use betterlite_core::api::responses_ws_history::ManagedResponsesWebSocket;
use betterlite_core::api::responses_ws_history::call_kwargs::build_base_call_kwargs;
use betterlite_core::api::responses_ws_history::history::{apply_history, extract_response_id};
use betterlite_core::api::responses_ws_history::messages::{
    extract_output_messages, input_to_messages,
};
use betterlite_core::api::responses_ws_history::serialization::{Chunk, serialize_chunk};
use betterlite_core::api::responses_ws_history::warmup::{
    WARMUP_RESPONSE_ID_PREFIX, is_warmup_frame, is_warmup_response_id, warmup_source_params,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde_json::{Number, Value, json};

use super::support::{Socket, frame_count, helper_error, sent_frames};
use super::{ApplyRow, HistoryCase};

fn serialized(kind: &str) -> Result<Json, Box<HelperError>> {
    match kind {
        "json" => {
            let mut fields = IndexMap::new();
            fields.insert("kind".to_owned(), Json::Text("json".to_owned()));
            fields.insert("n".to_owned(), Json::Number(Number::from(7_u8)));
            Ok(Json::Object(fields))
        }
        "text" => Ok(Json::Text("text".to_owned())),
        "number" => Ok(Json::Number(Number::from(7_u8))),
        "null" => Ok(Json::Null),
        "bool" => Ok(Json::Bool(true)),
        "list" => Ok(Json::List(vec![
            Json::Number(Number::from(1_u8)),
            Json::Number(Number::from(2_u8)),
        ])),
        other => Err(helper_error(
            "InvalidInput",
            None,
            format!("no serialized fixture for {other}"),
        )),
    }
}

pub(super) fn serialize_view(kinds: &[String]) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(kinds.len());
    for kind in kinds {
        let chunk = serialized(kind)?;
        answers.push(json!(serialize_chunk(Chunk::Json(&chunk))));
    }
    Ok(Value::Array(answers))
}

fn field(response: &Value, name: &str) -> Value {
    match response.get(name) {
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

fn response_view(response: &Value) -> Value {
    let identifier = response
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let created_at = response.get("created_at");
    let is_int = matches!(
        created_at,
        Some(Value::Number(number)) if number.is_i64() || number.is_u64()
    );
    json!({
        "id_is_warmup": identifier.starts_with(WARMUP_RESPONSE_ID_PREFIX),
        "id_hex": identifier.len() == WARMUP_RESPONSE_ID_PREFIX.len().saturating_add(32),
        "object": field(response, "object"),
        "created_at_is_int": is_int,
        "status": field(response, "status"),
        "model": field(response, "model"),
        "output": field(response, "output"),
        "usage": field(response, "usage"),
    })
}

fn ack_view(events: &[Value]) -> Value {
    let mut answers = Vec::with_capacity(events.len());
    for event in events {
        answers.push(json!({
            "type": field(event, "type"),
            "response": response_view(&field(event, "response")),
        }));
    }
    Value::Array(answers)
}

pub(super) async fn warmup_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    frames: &[IndexMap<String, Json>],
    ids: &[Option<String>],
) -> Result<Value, Box<HelperError>> {
    let mut acks = Vec::with_capacity(frames.len());
    for frame in frames {
        let start = frame_count(socket);
        handler
            .send_warmup_ack(frame)
            .await
            .map_err(|error| helper_error("TypeError", None, error.to_string()))?;
        acks.push(ack_view(&sent_frames(socket, start)?));
    }
    let mut flags = Vec::with_capacity(frames.len());
    let mut sources = Vec::with_capacity(frames.len());
    let mut responses = Vec::with_capacity(frames.len());
    for frame in frames {
        flags.push(json!(is_warmup_frame(frame)));
        sources.push(json!(warmup_source_params(frame)));
        responses.push(response_view(&json!(handler.build_warmup_response(frame))));
    }
    let mut id_flags = Vec::with_capacity(ids.len());
    for id in ids {
        id_flags.push(json!(is_warmup_response_id(id.as_deref())));
    }
    Ok(json!({
        "flags": flags,
        "ids": id_flags,
        "sources": sources,
        "responses": responses,
        "acks": acks,
    }))
}

pub(super) fn history_view(
    handler: &mut ManagedResponsesWebSocket<Socket>,
    history: &HistoryCase,
) -> Value {
    for stored in &history.store {
        handler.store_history(&stored.id, stored.messages.clone());
    }
    let first = handler.get_history_messages(&history.first);
    let mut appended = first.clone();
    appended.push(history.append.clone());
    let again = handler.get_history_messages(&history.first);
    json!({
        "first": first,
        "after_append": again,
        "missing": handler.get_history_messages(&history.missing),
    })
}

pub(super) fn inputs_view(values: &[Json]) -> Value {
    let mut answers = Vec::with_capacity(values.len());
    for value in values {
        answers.push(json!(input_to_messages(value)));
    }
    Value::Array(answers)
}

pub(super) fn completions_view(rows: &[IndexMap<String, Json>]) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let messages = extract_output_messages(row)
            .map_err(|error| helper_error("TypeError", None, error.to_string()))?;
        answers.push(json!({
            "id": extract_response_id(row),
            "messages": messages,
        }));
    }
    Ok(Value::Array(answers))
}

pub(super) fn base_kwargs_view(rows: &[IndexMap<String, Json>]) -> Value {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        answers.push(json!(build_base_call_kwargs(row)));
    }
    Value::Array(answers)
}

pub(super) fn apply_view(rows: &[ApplyRow]) -> Value {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let mut kwargs = IndexMap::new();
        apply_history(
            &mut kwargs,
            row.previous_response_id.as_deref(),
            &row.current,
            &row.prior,
        );
        answers.push(json!(kwargs));
    }
    Value::Array(answers)
}

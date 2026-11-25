use std::sync::atomic::Ordering;

use betterlite_core::api::responses_ws_history::ManagedResponsesWebSocket;
use betterlite_core::api::responses_ws_history::call_kwargs::build_base_call_kwargs;
use betterlite_core::api::responses_ws_history::history::{
    apply_history, extract_response_id, get_history_messages,
};
use betterlite_core::api::responses_ws_history::messages::{
    extract_output_messages, input_to_messages,
};
use betterlite_core::api::responses_ws_history::serialization::{
    Chunk, send_error, serialize_chunk,
};
use betterlite_core::api::responses_ws_history::warmup::{
    WARMUP_RESPONSE_ID_PREFIX, build_warmup_response, is_warmup_frame, is_warmup_response_id,
    send_warmup_ack, warmup_source_params,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::support::{Socket, helper_error};
use super::{Case, FrameRow};

pub(super) async fn frames_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    rows: &[FrameRow],
) -> Result<Value, Box<HelperError>> {
    let mut answers = Vec::with_capacity(rows.len());
    for row in rows {
        let start = socket.count();
        socket.fail_next.store(row.fail, Ordering::SeqCst);
        answers.push(frame_view(handler, socket, row, start).await?);
    }
    Ok(Value::Array(answers))
}

async fn frame_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    row: &FrameRow,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    match row.kind.as_str() {
        "serialize" => {
            let value = serde_json::from_str::<Json>(&row.payload)
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(json!(serialize_chunk(Chunk::Json(&value))))
        }
        "error" => {
            send_error(handler, &row.payload, row.error_type.as_deref()).await;
            Ok(json!(socket.since(start)?))
        }
        "parse" => {
            let parsed = handler.parse_message(&row.payload).await;
            let message =
                parsed.map_err(|error| helper_error("AttributeError", error.to_string()))?;
            Ok(parse_view(message, &socket.since(start)?))
        }
        other => Err(helper_error(
            "InvalidInput",
            format!("no frame kind {other}"),
        )),
    }
}

fn parse_view(message: Option<IndexMap<String, Json>>, frames: &[Value]) -> Value {
    let Some(fields) = message else {
        return json!({"kind": "ignored", "message": null, "frames": frames});
    };
    json!({"kind": "message", "message": Json::Object(fields), "frames": frames})
}

pub(super) async fn warmup_view(
    handler: &ManagedResponsesWebSocket<Socket>,
    socket: &Socket,
    frames: &[IndexMap<String, Json>],
    ids: &[Option<String>],
) -> Result<Value, Box<HelperError>> {
    let mut acks = Vec::with_capacity(frames.len());
    let mut flags = Vec::with_capacity(frames.len());
    let mut sources = Vec::with_capacity(frames.len());
    let mut responses = Vec::with_capacity(frames.len());
    for frame in frames {
        let start = socket.count();
        send_warmup_ack(handler, frame)
            .await
            .map_err(|error| helper_error("TypeError", error.to_string()))?;
        acks.push(ack_view(&socket.since(start)?));
        flags.push(json!(is_warmup_frame(frame)));
        sources.push(json!(warmup_source_params(frame)));
        responses.push(response_view(&json!(build_warmup_response(handler, frame))));
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

fn response_view(response: &Value) -> Value {
    let found = response.get("id").and_then(Value::as_str);
    let identifier = found.unwrap_or_default();
    json!({
        "id_is_warmup": identifier.starts_with(WARMUP_RESPONSE_ID_PREFIX),
        "id_hex": identifier.len() == WARMUP_RESPONSE_ID_PREFIX.len().saturating_add(32),
        "object": response.get("object").cloned().unwrap_or(Value::Null),
        "status": response.get("status").cloned().unwrap_or(Value::Null),
        "model": response.get("model").cloned().unwrap_or(Value::Null),
        "output": response.get("output").cloned().unwrap_or(Value::Null),
        "usage": response.get("usage").cloned().unwrap_or(Value::Null),
    })
}

fn ack_view(events: &[Value]) -> Value {
    let mut answers = Vec::with_capacity(events.len());
    for event in events {
        answers.push(json!({
            "type": event.get("type").cloned().unwrap_or(Value::Null),
            "response": response_view(&event.get("response").cloned().unwrap_or(Value::Null)),
        }));
    }
    Value::Array(answers)
}

pub(super) fn messages_view(case: &Case) -> Result<Value, Box<HelperError>> {
    let mut inputs = Vec::with_capacity(case.inputs.len());
    for value in &case.inputs {
        inputs.push(json!(input_to_messages(value)));
    }
    let mut completions = Vec::with_capacity(case.completions.len());
    for row in &case.completions {
        let messages = extract_output_messages(row)
            .map_err(|error| helper_error("TypeError", error.to_string()))?;
        completions.push(json!({"id": extract_response_id(row), "messages": messages}));
    }
    Ok(json!({"inputs": inputs, "completions": completions}))
}

pub(super) fn history_view(handler: &mut ManagedResponsesWebSocket<Socket>, case: &Case) -> Value {
    for stored in &case.store {
        handler.store_history(stored.id.clone(), stored.messages.clone());
    }
    let mut first = get_history_messages(handler, &case.first);
    first.push(case.append.clone());
    let again = get_history_messages(handler, &case.first);
    let mut applied = Vec::with_capacity(case.applied.len());
    for row in &case.applied {
        let mut kwargs = IndexMap::new();
        apply_history(
            &mut kwargs,
            row.previous_response_id.as_deref(),
            &row.current,
            &row.prior,
        );
        applied.push(json!(kwargs));
    }
    let mut base_kwargs = Vec::with_capacity(case.base_kwargs.len());
    for row in &case.base_kwargs {
        base_kwargs.push(json!(build_base_call_kwargs(row)));
    }
    json!({
        "first": first,
        "after_append": again,
        "missing": get_history_messages(handler, &case.missing),
        "applied": applied,
        "base_kwargs": base_kwargs,
    })
}

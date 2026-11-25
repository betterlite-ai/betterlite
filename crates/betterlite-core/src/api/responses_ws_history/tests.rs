#![cfg(test)]
#![expect(
    clippy::cognitive_complexity,
    clippy::shadow_unrelated,
    clippy::too_many_lines,
    reason = "each named case asserts a whole frame or projection sequence, and a case rebinds its handler as it moves to the next scenario"
)]

mod support;

use std::collections::BTreeMap;
use std::slice::from_ref;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::response_synthetic_events::error::SyntheticEventsError;
use crate::api::response_synthetic_events::model_dump::{HasModelDump, HasModelDumpJson};
use crate::api::responses_ws_history::call_kwargs::{
    build_base_call_kwargs, inject_credentials, response_create_params, update_proxy_request,
};
use crate::api::responses_ws_history::error::ResponsesWsHistoryError;
use crate::api::responses_ws_history::history::{
    apply_history, extract_response_id, get_history_messages,
};
use crate::api::responses_ws_history::messages::{extract_output_messages, input_to_messages};
use crate::api::responses_ws_history::provider::{resolve_provider, same_provider};
use crate::api::responses_ws_history::serialization::{
    Chunk, ChunkWriter, send_error, serialize_chunk,
};
use crate::api::responses_ws_history::warmup::{
    build_warmup_response, is_warmup_frame, is_warmup_response_id, send_warmup_ack,
    warmup_source_params,
};
use crate::api::responses_ws_history::{MANAGED_WS_SKIP_KWARGS, ManagedResponsesWebSocket};

use support::{ENCODED_RAW_ONE, MODEL, NOW, OTHER_MODEL, UNKNOWN_MODEL, WARMUP_ID_HEX};

fn object_map(value: &Json) -> IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is not an object: {value:?}")
        }
    }
}

fn as_json(messages: Vec<IndexMap<String, Json>>) -> Json {
    let items = messages.into_iter().map(Json::Object).collect();
    Json::List(items)
}

fn warmup_event(event_type: &str, status: &str, model: &str) -> String {
    format!(
        r#"{{"type": "{event_type}", "response": {{"id": "resp_warmup_{WARMUP_ID_HEX}", "object": "response", "created_at": {NOW}, "status": "{status}", "model": "{model}", "output": [], "usage": {{"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}}}}}}"#
    )
}

fn warmup_response(status: &str, model: &str) -> Json {
    match support::json(&warmup_event("response.created", status, model)) {
        Json::Object(mut fields) => fields.shift_remove("response").unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the warmup event is not an object")
        }
    }
}

struct DumpJson(&'static str);

impl HasModelDumpJson for DumpJson {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(self.0.to_owned())
    }
}

struct DumpOnly(BTreeMap<String, Json>);

impl HasModelDump for DumpOnly {
    fn model_dump(&self) -> Result<BTreeMap<String, Json>, SyntheticEventsError> {
        Ok(self.0.clone())
    }
}

struct Both {
    text: &'static str,
    fields: BTreeMap<String, Json>,
}

impl HasModelDumpJson for Both {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Ok(self.text.to_owned())
    }
}

impl HasModelDump for Both {
    fn model_dump(&self) -> Result<BTreeMap<String, Json>, SyntheticEventsError> {
        Ok(self.fields.clone())
    }
}

struct FailingJson;

impl HasModelDumpJson for FailingJson {
    fn model_dump_json(&self) -> Result<String, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: "chunk" })
    }
}

struct FailingDump;

impl HasModelDump for FailingDump {
    fn model_dump(&self) -> Result<BTreeMap<String, Json>, SyntheticEventsError> {
        Err(SyntheticEventsError::NotAnObject { field: "chunk" })
    }
}

fn fail_created(chunk: Chunk<'_>) -> Option<String> {
    if let Chunk::Json(Json::Object(fields)) = chunk
        && matches!(
            fields.get("type"),
            Some(Json::Text(event_type)) if event_type == "response.created"
        )
    {
        return None;
    }
    serialize_chunk(chunk)
}

fn model_group_view(metadata: &str) -> Option<String> {
    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.litellm_metadata = Some(object_map(&support::json(metadata)));
    let handler = support::plain(connection);
    assert_eq!(
        Json::Object(handler.litellm_metadata.clone()),
        support::json(metadata),
        "the metadata map is stored unchanged"
    );
    handler.model_group
}

fn warmup_frame(text: &str) -> bool {
    is_warmup_frame(&object_map(&support::json(text)))
}

async fn assert_non_object(handler: &ManagedResponsesWebSocket<support::RecordingSink>, raw: &str) {
    match handler.parse_message(raw).await {
        Err(ResponsesWsHistoryError::Shape { field, expected }) => {
            assert_eq!(field, "message");
            assert_eq!(expected, "a JSON object");
        }
        other => panic!("{raw} must be refused as a non-object, got {other:?}"),
    }
}

fn base_kwargs_view(text: &str) -> Json {
    Json::Object(build_base_call_kwargs(&object_map(&support::json(text))))
}

fn response_id(text: &str) -> Option<String> {
    extract_response_id(&object_map(&support::json(text)))
}

fn proxy_fresh(text: &str) -> (Option<Json>, Option<Json>) {
    let mut fresh = object_map(&support::json(text));
    update_proxy_request(&mut fresh, "m").expect("the fresh proxy request updates");
    let stored = fresh.get("proxy_server_request").cloned();
    let nested = fresh.get("litellm_metadata").and_then(|value| match value {
        Json::Object(fields) => fields.get("proxy_server_request").cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    });
    (stored, nested)
}

fn proxy_unchanged(text: &str) -> bool {
    let original = object_map(&support::json(text));
    let mut kwargs = original.clone();
    update_proxy_request(&mut kwargs, "m").expect("a truthy non-object request is ignored");
    Json::Object(kwargs) == Json::Object(original)
}

#[test]
fn constructor_defaults_and_passthrough() {
    assert_eq!(
        MANAGED_WS_SKIP_KWARGS,
        [
            "litellm_logging_obj",
            "litellm_call_id",
            "aresponses",
            "_aresponses_websocket",
            "user_api_key_dict",
        ]
    );
    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.kwargs.extend(
        MANAGED_WS_SKIP_KWARGS
            .into_iter()
            .map(|name| (name.to_owned(), Json::Text("drop".to_owned()))),
    );
    connection.kwargs.insert(
        "extra_headers".to_owned(),
        support::json(r#"{"X-Test": "yes"}"#),
    );
    let handler = support::plain(connection);
    assert_eq!(handler.model, MODEL);
    assert_eq!(handler.litellm_metadata, IndexMap::new());
    assert_eq!(handler.model_group, None);
    assert_eq!(handler.connection_provider.as_deref(), Some("openai"));
    assert_eq!(handler.first_message, None);
    assert!(handler.api_key.is_none());
    assert_eq!(handler.api_base, None);
    assert_eq!(handler.timeout, None);
    assert_eq!(handler.custom_llm_provider, None);
    assert!(handler.user_api_key_dict.is_none());
    assert!(handler.quota_callbacks.is_empty());
    assert!(get_history_messages(&handler, "raw-0").is_empty());
    assert_eq!(
        Json::Object(handler.extra_kwargs),
        support::json(r#"{"extra_headers": {"X-Test": "yes"}}"#)
    );

    assert_eq!(
        model_group_view(
            r#"{"model_group": "router-alias", "deployment_model_name": "deployment-alias"}"#
        )
        .as_deref(),
        Some("router-alias")
    );
    assert_eq!(
        model_group_view(r#"{"model_group": "", "deployment_model_name": "deployment-alias"}"#)
            .as_deref(),
        Some("deployment-alias")
    );
    assert_eq!(
        model_group_view(r#"{"model_group": 7, "deployment_model_name": "deployment-alias"}"#)
            .as_deref(),
        None
    );
    assert_eq!(
        model_group_view(r#"{"model_group": [], "deployment_model_name": 7}"#).as_deref(),
        None
    );
    assert_eq!(
        model_group_view(r#"{"model_group": {}, "deployment_model_name": "deployment-alias"}"#)
            .as_deref(),
        Some("deployment-alias")
    );
    assert_eq!(
        model_group_view(r#"{"deployment_model_name": "deployment-alias"}"#).as_deref(),
        Some("deployment-alias")
    );
    assert_eq!(model_group_view("{}").as_deref(), None);

    let recording = support::recording_sink();
    let mut connection = support::connection(UNKNOWN_MODEL, recording.sink);
    connection.custom_llm_provider = Some("proxy".to_owned());
    let handler = support::plain(connection);
    assert_eq!(handler.connection_provider.as_deref(), Some("proxy"));

    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.first_message = Some("hello".to_owned());
    connection.quota_callbacks = Some(Vec::new());
    let handler = support::plain(connection);
    assert_eq!(handler.first_message.as_deref(), Some("hello"));
    assert!(handler.quota_callbacks.is_empty());

    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.quota_callbacks = Some(vec![
        Arc::new(support::TestQuota),
        Arc::new(support::TestQuota),
    ]);
    assert_eq!(support::plain(connection).quota_callbacks.len(), 2);
}

#[tokio::test]
async fn serialization_and_error_drop() {
    let dumped = DumpJson(r#"{"kind":"json"}"#);
    assert_eq!(
        serialize_chunk(Chunk::ModelDumpJson(&dumped)).as_deref(),
        Some(r#"{"kind":"json"}"#)
    );
    let mut fields = BTreeMap::new();
    fields.insert("kind".to_owned(), Json::Text("dump".to_owned()));
    fields.insert("n".to_owned(), Json::Number(Number::from(7_u8)));
    let dump_only = DumpOnly(fields.clone());
    assert_eq!(
        serialize_chunk(Chunk::ModelDump(&dump_only)).as_deref(),
        Some(r#"{"kind": "dump", "n": 7}"#)
    );
    let both = Both {
        text: r#"{"from":"json"}"#,
        fields: fields.clone(),
    };
    assert_eq!(
        serialize_chunk(Chunk::ModelDumpJson(&both)).as_deref(),
        Some(r#"{"from":"json"}"#)
    );
    assert_eq!(
        serialize_chunk(Chunk::ModelDump(&both)).as_deref(),
        Some(r#"{"kind": "dump", "n": 7}"#)
    );

    let object = support::json(r#"{"n": 7}"#);
    assert_eq!(
        serialize_chunk(Chunk::Json(&object)).as_deref(),
        Some(r#"{"n": 7}"#)
    );
    let number = Json::Number(Number::from(7_u8));
    assert_eq!(
        serialize_chunk(Chunk::Json(&number)).as_deref(),
        Some(r#""7""#)
    );
    let nothing = Json::Null;
    assert_eq!(
        serialize_chunk(Chunk::Json(&nothing)).as_deref(),
        Some(r#""None""#)
    );
    let text = Json::Text("text".to_owned());
    assert_eq!(
        serialize_chunk(Chunk::Json(&text)).as_deref(),
        Some(r#""text""#)
    );
    let flag = Json::Bool(true);
    assert_eq!(
        serialize_chunk(Chunk::Json(&flag)).as_deref(),
        Some(r#""True""#)
    );

    let ((failed_json, failed_dump), logs) = support::capture(|| {
        (
            serialize_chunk(Chunk::ModelDumpJson(&FailingJson)),
            serialize_chunk(Chunk::ModelDump(&FailingDump)),
        )
    });
    assert_eq!((failed_json, failed_dump), (None, None));
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: failed to serialize chunk: chunk must be a JSON object",
            "ManagedResponsesWS: failed to serialize chunk: chunk must be a JSON object",
        ]
    );

    let recording = support::recording_sink();
    let frames = Arc::clone(&recording.frames);
    let handler = support::plain(support::connection(MODEL, recording.sink));
    send_error(&handler, "quota exceeded", None).await;
    send_error(&handler, "bad", Some("invalid_request_error")).await;
    assert_eq!(
        support::sent(&frames),
        vec![
            r#"{"type": "error", "error": {"type": "server_error", "message": "quota exceeded"}}"#,
            r#"{"type": "error", "error": {"type": "invalid_request_error", "message": "bad"}}"#,
        ]
    );

    recording.fail_next.store(true, Ordering::SeqCst);
    let before = support::sent(&frames).len();
    send_error(&handler, "lost", None).await;
    assert_eq!(support::sent(&frames).len(), before);
}

#[tokio::test]
async fn parse_message_cases() {
    let recording = support::recording_sink();
    let frames = Arc::clone(&recording.frames);
    let handler = support::plain(support::connection(MODEL, recording.sink));

    let dropped = handler.parse_message("not-json").await;
    assert!(matches!(dropped, Ok(None)));
    assert_eq!(
        support::sent(&frames),
        vec![
            r#"{"type": "error", "error": {"type": "invalid_request_error", "message": "Invalid JSON in response.create event"}}"#
        ]
    );

    let ping = handler.parse_message(r#"{"type": "ping"}"#).await;
    assert!(matches!(ping, Ok(None)));
    assert_eq!(support::sent(&frames).len(), 1);

    assert_non_object(&handler, "[]").await;
    assert_non_object(&handler, "null").await;
    assert_non_object(&handler, "7").await;
    assert_non_object(&handler, "\"text\"").await;
    assert_eq!(support::sent(&frames).len(), 1);

    let Ok(Some(parsed)) = handler
        .parse_message(r#"{"type": "response.create", "input": "hello"}"#)
        .await
    else {
        panic!("a response.create object parses");
    };
    assert_eq!(
        Json::Object(parsed),
        support::json(r#"{"type": "response.create", "input": "hello"}"#)
    );
    let Ok(nothing) = handler.parse_message(r#"{"type": 7}"#).await else {
        panic!("a non-string type is a normal message");
    };
    assert!(nothing.is_none());

    recording.fail_next.store(true, Ordering::SeqCst);
    let swallowed = handler.parse_message("not-json").await;
    assert!(matches!(swallowed, Ok(None)));
    assert_eq!(support::sent(&frames).len(), 1);
}

#[tokio::test]
async fn warmup_ack_without_provider() {
    assert!(warmup_frame(
        r#"{"type": "response.create", "generate": false}"#
    ));
    assert!(warmup_frame(
        r#"{"type": "response.create", "response": {"generate": false}}"#
    ));
    assert!(warmup_frame(
        r#"{"type": "response.create", "generate": false, "response": {}}"#
    ));
    assert!(!warmup_frame(
        r#"{"type": "response.create", "generate": 0}"#
    ));
    assert!(!warmup_frame(r#"{"type": "response.create"}"#));
    assert!(!warmup_frame(
        r#"{"type": "response.create", "response": {"generate": 0}}"#
    ));
    assert!(!is_warmup_response_id(None));
    assert!(!is_warmup_response_id(Some("")));
    assert!(!is_warmup_response_id(Some("raw-1")));
    let warmup_id = format!("resp_warmup_{WARMUP_ID_HEX}");
    assert!(is_warmup_response_id(Some(&warmup_id)));

    let nested = object_map(&support::json(
        r#"{"type": "response.create", "a": 1, "response": {"b": 2}}"#,
    ));
    assert_eq!(
        Json::Object(warmup_source_params(&nested)),
        support::json(r#"{"b": 2}"#)
    );
    let flat = object_map(&support::json(r#"{"type": "response.create", "a": 1}"#));
    assert_eq!(
        Json::Object(warmup_source_params(&flat)),
        support::json(r#"{"a": 1}"#)
    );

    let recording = support::recording_sink();
    let frames = Arc::clone(&recording.frames);
    let resolver = Arc::new(support::ScriptedResolver);
    let mut connection = support::connection(MODEL, recording.sink);
    connection.litellm_metadata = Some(object_map(&support::json(r#"{"model_group": "group-1"}"#)));
    let handler = support::handler(connection, Arc::clone(&resolver), support::serializer());

    let built = build_warmup_response(
        &handler,
        &object_map(&support::json(r#"{"type": "response.create"}"#)),
    );
    assert_eq!(Json::Object(built), warmup_response("completed", "group-1"));
    let sourced = build_warmup_response(
        &handler,
        &object_map(&support::json(
            r#"{"type": "response.create", "model": "m2"}"#,
        )),
    );
    assert_eq!(sourced.get("model"), Some(&Json::Text("m2".to_owned())));

    send_warmup_ack(
        &handler,
        &object_map(&support::json(
            r#"{"type": "response.create", "model": "m3"}"#,
        )),
    )
    .await
    .unwrap();
    assert_eq!(
        support::sent(&frames),
        vec![
            warmup_event("response.created", "in_progress", "m3"),
            warmup_event("response.completed", "completed", "m3"),
        ]
    );

    let recording = support::recording_sink();
    let frames = Arc::clone(&recording.frames);
    let serializer: Arc<ChunkWriter> = Arc::new(fail_created);
    let handler = support::handler(
        support::connection(MODEL, recording.sink),
        Arc::new(support::ScriptedResolver),
        serializer,
    );
    send_warmup_ack(
        &handler,
        &object_map(&support::json(r#"{"type": "response.create"}"#)),
    )
    .await
    .unwrap();
    assert_eq!(
        support::sent(&frames),
        vec![warmup_event("response.completed", "completed", MODEL)]
    );
}

#[test]
fn input_and_output_message_projection() {
    let hello = input_to_messages(&support::json("\"hello\""));
    assert_eq!(
        as_json(hello),
        support::json(
            r#"[{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]}]"#
        )
    );
    let array = input_to_messages(&support::json(
        r#"[{"type": "message", "role": "user"}, 7, "drop"]"#,
    ));
    assert_eq!(
        as_json(array),
        support::json(r#"[{"type": "message", "role": "user"}]"#)
    );
    assert!(input_to_messages(&support::json("null")).is_empty());
    assert!(input_to_messages(&support::json("{}")).is_empty());
    assert!(input_to_messages(&support::json("7")).is_empty());
    assert!(input_to_messages(&support::json("[]")).is_empty());
    assert!(input_to_messages(&support::json(r#"{"type": "message"}"#)).is_empty());

    let completed = object_map(&support::json(
        r#"{"response": {"output": [
            {"type": "message", "content": [
                {"type": "output_text", "text": "A"},
                {"type": "text", "text": "B"},
                {"type": "input_text", "text": "C"}]},
            {"type": "function_call", "name": "f"},
            {"type": "message", "role": null, "content": [{"type": "output_text", "text": "z"}]},
            {"type": "message", "role": "user", "content": [{"type": "output_text", "text": ""}]},
            {"type": "other"}
        ]}}"#,
    ));
    let projected = extract_output_messages(&completed).unwrap();
    assert_eq!(
        as_json(projected),
        support::json(
            r#"[
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "AB"}]},
                {"type": "function_call", "name": "f"},
                {"type": "message", "role": null, "content": [{"type": "output_text", "text": "z"}]}
            ]"#
        )
    );
    assert!(
        extract_output_messages(&object_map(&support::json("{}")))
            .unwrap()
            .is_empty()
    );
    assert!(
        extract_output_messages(&object_map(&support::json(r#"{"response": null}"#)))
            .unwrap()
            .is_empty()
    );
    assert!(
        extract_output_messages(&object_map(&support::json(
            r#"{"response": {"output": []}}"#
        )))
        .unwrap()
        .is_empty()
    );
    match extract_output_messages(&object_map(&support::json(
        r#"{"response": {"output": [{"type": "message", "content": [{"type": "output_text", "text": 7}]}]}}"#,
    ))) {
        Err(ResponsesWsHistoryError::Shape { field, expected }) => {
            assert_eq!(field, "text");
            assert_eq!(expected, "a string");
        }
        other => panic!("an integer text must be refused, got {other:?}"),
    }
}

#[test]
fn response_id_history() {
    assert_eq!(
        extract_response_id(&object_map(&support::json(&format!(
            r#"{{"response": {{"id": "{ENCODED_RAW_ONE}"}}}}"#
        )))),
        Some("raw-1".to_owned())
    );
    assert_eq!(response_id("{}"), None);
    assert_eq!(response_id(r#"{"response": "not-a-dict"}"#), None);
    assert_eq!(response_id(r#"{"response": {}}"#), None);
    assert_eq!(response_id(r#"{"response": {"id": null}}"#), None);
    assert_eq!(response_id(r#"{"response": {"id": 7}}"#), None);
    assert_eq!(response_id(r#"{"response": {"id": ""}}"#), None);

    let recording = support::recording_sink();
    let mut handler = support::plain(support::connection(MODEL, recording.sink));
    assert!(get_history_messages(&handler, ENCODED_RAW_ONE).is_empty());
    let message_a = object_map(&support::json(
        r#"{"type": "message", "role": "user", "content": []}"#,
    ));
    handler.store_history("raw-1".to_owned(), vec![message_a.clone()]);
    let mut got = get_history_messages(&handler, ENCODED_RAW_ONE);
    assert_eq!(got, vec![message_a.clone()]);
    let marker = object_map(&support::json(r#"{"probe": true}"#));
    got.push(marker);
    assert_eq!(
        get_history_messages(&handler, ENCODED_RAW_ONE),
        vec![message_a]
    );

    let message_b = object_map(&support::json(
        r#"{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "B"}]}"#,
    ));
    handler.store_history("raw-1".to_owned(), vec![message_b.clone()]);
    assert_eq!(
        get_history_messages(&handler, ENCODED_RAW_ONE),
        vec![message_b]
    );
    assert!(get_history_messages(&handler, "raw-9").is_empty());
}

#[test]
fn base_call_kwargs() {
    assert_eq!(
        response_create_params().collect::<Vec<&str>>(),
        [
            "input",
            "model",
            "include",
            "instructions",
            "max_output_tokens",
            "metadata",
            "parallel_tool_calls",
            "previous_response_id",
            "reasoning",
            "store",
            "background",
            "stream",
            "temperature",
            "text",
            "tool_choice",
            "tools",
            "top_p",
            "truncation",
            "user",
            "service_tier",
            "safety_identifier",
            "prompt",
            "max_tool_calls",
            "prompt_cache_key",
            "prompt_cache_retention",
            "prompt_cache_options",
            "stream_options",
            "top_logprobs",
            "partial_images",
            "context_management",
        ]
    );
    let flat = object_map(&support::json(
        r#"{"type": "response.create", "input": "hello", "model": "openai/gpt-4o-mini", "store": false, "unknown": 1}"#,
    ));
    assert_eq!(
        Json::Object(build_base_call_kwargs(&flat)),
        support::json(r#"{"input": "hello", "model": "openai/gpt-4o-mini", "store": false}"#)
    );
    let nulls = object_map(&support::json(
        r#"{"type": "response.create", "input": null, "model": null, "store": null, "tools": null}"#,
    ));
    assert_eq!(
        Json::Object(build_base_call_kwargs(&nulls)),
        support::json("{}")
    );
    let nested = object_map(&support::json(
        r#"{"type": "response.create", "input": "flat", "model": "flat-m", "response": {"input": "nested", "model": "nested-m"}}"#,
    ));
    assert_eq!(
        Json::Object(build_base_call_kwargs(&nested)),
        support::json(r#"{"input": "nested", "model": "nested-m"}"#)
    );
    assert_eq!(
        base_kwargs_view(r#"{"type": "response.create", "input": "flat", "response": {}}"#),
        support::json(r#"{"input": "flat"}"#)
    );
    assert_eq!(
        base_kwargs_view(r#"{"type": "response.create", "input": "flat", "response": null}"#),
        support::json(r#"{"input": "flat"}"#)
    );
    assert_eq!(
        base_kwargs_view(r#"{"type": "response.create", "input": "flat", "response": "x"}"#),
        support::json(r#"{"input": "flat"}"#)
    );
    let mut every = IndexMap::new();
    every.insert("type".to_owned(), Json::Text("response.create".to_owned()));
    every.extend(
        response_create_params().map(|name| (name.to_owned(), Json::Text(format!("v-{name}")))),
    );
    every.insert("unknown".to_owned(), Json::Text("x".to_owned()));
    let built = build_base_call_kwargs(&every);
    assert_eq!(built.len(), 30);
    assert!(!built.contains_key("unknown"));
    assert!(!built.contains_key("type"));
    assert_eq!(built.get("input"), Some(&Json::Text("v-input".to_owned())));
}

#[test]
fn apply_history_paths() {
    let prior = object_map(&support::json(
        r#"{"type": "message", "role": "user", "content": []}"#,
    ));
    let current = object_map(&support::json(
        r#"{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "next"}]}"#,
    ));
    let mut untouched = IndexMap::new();
    untouched.insert("keep".to_owned(), Json::Number(Number::from(1_u8)));
    let original = untouched.clone();
    apply_history(&mut untouched, None, from_ref(&current), from_ref(&prior));
    apply_history(&mut untouched, Some(""), &[], &[]);
    assert_eq!(untouched, original);

    let ((), logs) = support::capture(|| {
        let mut kwargs = IndexMap::new();
        apply_history(
            &mut kwargs,
            Some("raw-1"),
            from_ref(&current),
            from_ref(&prior),
        );
        assert_eq!(
            Json::Object(kwargs),
            support::json(
                r#"{"input": [
                    {"type": "message", "role": "user", "content": []},
                    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "next"}]}
                ]}"#
            )
        );
    });
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: prepended 1 history messages for previous_response_id=raw-1"]
    );

    let ((), logs) = support::capture(|| {
        let mut kwargs = IndexMap::new();
        kwargs.insert("input".to_owned(), Json::Text("kept".to_owned()));
        apply_history(&mut kwargs, Some("raw-2"), &[], &[]);
        assert_eq!(
            Json::Object(kwargs),
            support::json(r#"{"input": "kept", "previous_response_id": "raw-2"}"#)
        );
    });
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: no in-memory history for previous_response_id=raw-2; falling back to DB-based session reconstruction"
        ]
    );

    let warmup_id = format!("resp_warmup_{WARMUP_ID_HEX}");
    let ((), logs) = support::capture(|| {
        let mut kwargs = IndexMap::new();
        kwargs.insert("keep".to_owned(), Json::Number(Number::from(1_u8)));
        apply_history(
            &mut kwargs,
            Some(&warmup_id),
            from_ref(&current),
            from_ref(&prior),
        );
        assert_eq!(Json::Object(kwargs), support::json(r#"{"keep": 1}"#));
    });
    assert_eq!(
        logs,
        vec![format!(
            "ManagedResponsesWS: ignoring synthetic warmup previous_response_id={warmup_id}"
        )]
    );
}

#[test]
fn same_provider_credentials() {
    let resolver = Arc::new(support::ScriptedResolver);
    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.api_key = Some(support::key("key-1"));
    connection.api_base = Some("https://api.example.test".to_owned());
    connection.timeout = Some(7.5_f64);
    connection.custom_llm_provider = Some("proxy".to_owned());
    connection.litellm_metadata = Some(object_map(&support::json(r#"{"trace_id": "t1"}"#)));
    let handler = support::handler(connection, Arc::clone(&resolver), support::serializer());

    assert_eq!(
        resolve_provider(&handler, Some(MODEL)).as_deref(),
        Some("openai")
    );
    assert_eq!(
        resolve_provider(&handler, Some(OTHER_MODEL)).as_deref(),
        Some("anthropic")
    );
    assert_eq!(resolve_provider(&handler, Some("")), None);
    assert_eq!(resolve_provider(&handler, Some(UNKNOWN_MODEL)), None);
    assert_eq!(resolve_provider(&handler, None), None);

    assert!(same_provider(&handler, None));
    assert!(same_provider(&handler, Some(MODEL)));
    assert!(same_provider(&handler, Some("openai/gpt-4o")));
    assert!(!same_provider(&handler, Some(OTHER_MODEL)));
    assert!(!same_provider(&handler, Some(UNKNOWN_MODEL)));

    let mut kwargs = IndexMap::new();
    inject_credentials(&handler, &mut kwargs, Some("openai/gpt-4o")).unwrap();
    assert_eq!(
        Json::Object(kwargs),
        support::json(
            r#"{"api_key": "key-1", "api_base": "https://api.example.test", "timeout": 7.5, "custom_llm_provider": "proxy", "litellm_metadata": {"trace_id": "t1"}}"#
        )
    );

    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.api_base = Some("https://api.example.test".to_owned());
    let handler = support::plain(connection);
    let mut independent = IndexMap::new();
    inject_credentials(&handler, &mut independent, None).unwrap();
    assert_eq!(
        Json::Object(independent),
        support::json(r#"{"api_base": "https://api.example.test"}"#)
    );

    let recording = support::recording_sink();
    let handler = support::plain(support::connection(MODEL, recording.sink));
    let mut empty = IndexMap::new();
    inject_credentials(&handler, &mut empty, None).unwrap();
    assert!(empty.is_empty());

    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.timeout = Some(f64::NAN);
    let handler = support::plain(connection);
    let mut infinite = IndexMap::new();
    match inject_credentials(&handler, &mut infinite, None) {
        Err(ResponsesWsHistoryError::Shape { field, expected }) => {
            assert_eq!(field, "timeout");
            assert_eq!(expected, "a finite number");
        }
        other => panic!("a non-finite timeout must be refused, got {other:?}"),
    }
}

#[test]
fn different_provider_does_not_copy_credentials() {
    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.api_key = Some(support::key("key-1"));
    connection.api_base = Some("https://api.example.test".to_owned());
    connection.timeout = Some(7.5_f64);
    connection.custom_llm_provider = Some("proxy".to_owned());
    connection.litellm_metadata = Some(object_map(&support::json(r#"{"trace_id": "t1"}"#)));
    let handler = support::plain(connection);
    assert!(!same_provider(&handler, Some(OTHER_MODEL)));
    let mut kwargs = IndexMap::new();
    inject_credentials(&handler, &mut kwargs, Some(OTHER_MODEL)).unwrap();
    assert_eq!(
        Json::Object(kwargs.clone()),
        support::json(
            r#"{"api_key": "key-1", "api_base": "https://api.example.test", "timeout": 7.5, "litellm_metadata": {"trace_id": "t1"}}"#
        )
    );
    assert!(!kwargs.contains_key("custom_llm_provider"));

    let recording = support::recording_sink();
    let mut connection = support::connection(UNKNOWN_MODEL, recording.sink);
    connection.api_key = Some(support::key("key-2"));
    connection.custom_llm_provider = Some("proxy".to_owned());
    let handler = support::plain(connection);
    assert_eq!(handler.connection_provider.as_deref(), Some("proxy"));
    let mut kwargs = IndexMap::new();
    inject_credentials(&handler, &mut kwargs, Some(OTHER_MODEL)).unwrap();
    assert_eq!(
        kwargs.get("api_key").cloned(),
        Some(Json::Text("key-2".to_owned()))
    );
    assert!(!kwargs.contains_key("custom_llm_provider"));
    assert!(
        same_provider(&handler, Some(UNKNOWN_MODEL)),
        "the connection's own model is the same provider even when its provider is the custom fallback"
    );

    let recording = support::recording_sink();
    let mut connection = support::connection(MODEL, recording.sink);
    connection.custom_llm_provider = Some("proxy".to_owned());
    let handler = support::plain(connection);
    let mut kwargs = IndexMap::new();
    inject_credentials(&handler, &mut kwargs, Some("")).unwrap();
    assert!(!kwargs.contains_key("custom_llm_provider"));
    assert!(!kwargs.contains_key("api_key"));
}

#[test]
fn proxy_request_projection() {
    let mut kwargs = object_map(&support::json(
        r#"{"input": ["hello"], "store": null, "tools": null, "tool_choice": "auto", "instructions": null, "metadata": {"tenant": "t1"}}"#,
    ));
    update_proxy_request(&mut kwargs, MODEL).unwrap();
    let request = support::json(
        r#"{"body": {"input": ["hello"], "store": null, "model": "openai/gpt-4o-mini", "tool_choice": "auto", "metadata": {"tenant": "t1"}}}"#,
    );
    let mut expected = object_map(&support::json(
        r#"{"input": ["hello"], "store": null, "tools": null, "tool_choice": "auto", "instructions": null, "metadata": {"tenant": "t1"}}"#,
    ));
    let mut metadata = IndexMap::new();
    metadata.insert("proxy_server_request".to_owned(), request.clone());
    expected.insert("litellm_metadata".to_owned(), Json::Object(metadata));
    expected.insert("proxy_server_request".to_owned(), request);
    assert_eq!(Json::Object(kwargs), Json::Object(expected));

    let mut existing = object_map(&support::json(
        r#"{"input": "i", "litellm_metadata": {"trace_id": "t1", "proxy_server_request": {"body": {"old": 1}, "keep": 2}}}"#,
    ));
    update_proxy_request(&mut existing, "m").unwrap();
    let updated = support::json(
        r#"{"body": {"old": 1, "input": "i", "store": null, "model": "m"}, "keep": 2}"#,
    );
    assert_eq!(
        existing.get("litellm_metadata").map(object_map),
        Some({
            let mut metadata = object_map(&support::json(
                r#"{"trace_id": "t1", "proxy_server_request": {"body": {"old": 1}}}"#,
            ));
            metadata.insert("proxy_server_request".to_owned(), updated.clone());
            metadata
        })
    );
    assert_eq!(existing.get("proxy_server_request").cloned(), Some(updated));

    let body = support::json(r#"{"body": {"input": "i", "store": null, "model": "m"}}"#);
    assert_eq!(
        proxy_fresh(r#"{"input": "i", "litellm_metadata": null}"#),
        (Some(body.clone()), Some(body.clone()))
    );
    assert_eq!(
        proxy_fresh(r#"{"input": "i", "litellm_metadata": {"proxy_server_request": null}}"#),
        (Some(body.clone()), Some(body.clone()))
    );
    assert_eq!(
        proxy_fresh(r#"{"input": "i", "litellm_metadata": {"proxy_server_request": 0}}"#),
        (Some(body.clone()), Some(body.clone()))
    );
    assert_eq!(
        proxy_fresh(r#"{"input": "i", "litellm_metadata": {"proxy_server_request": {}}}"#),
        (Some(body.clone()), Some(body.clone()))
    );
    assert_eq!(
        proxy_fresh(r#"{"input": "i", "litellm_metadata": {"proxy_server_request": []}}"#),
        (Some(body.clone()), Some(body.clone()))
    );
    assert_eq!(
        proxy_fresh(
            r#"{"input": "i", "litellm_metadata": {"proxy_server_request": {"body": null}}}"#
        ),
        (Some(body.clone()), Some(body))
    );
    assert!(proxy_unchanged(
        r#"{"input": "i", "litellm_metadata": {"proxy_server_request": "not-a-dict"}}"#
    ));
    assert!(proxy_unchanged(
        r#"{"input": "i", "litellm_metadata": {"proxy_server_request": [1]}}"#
    ));

    let mut literal_metadata = object_map(&support::json(
        r#"{"input": "i", "litellm_metadata": "not-a-dict"}"#,
    ));
    match update_proxy_request(&mut literal_metadata, "m") {
        Err(ResponsesWsHistoryError::Shape { field, expected }) => {
            assert_eq!(field, "litellm_metadata");
            assert_eq!(expected, "an object");
        }
        other => panic!("a non-object metadata must be refused, got {other:?}"),
    }
    let mut literal_body = object_map(&support::json(
        r#"{"input": "i", "litellm_metadata": {"proxy_server_request": {"body": "text"}}}"#,
    ));
    match update_proxy_request(&mut literal_body, "m") {
        Err(ResponsesWsHistoryError::Shape { field, expected }) => {
            assert_eq!(field, "body");
            assert_eq!(expected, "an object");
        }
        other => panic!("a non-object body must be refused, got {other:?}"),
    }
}

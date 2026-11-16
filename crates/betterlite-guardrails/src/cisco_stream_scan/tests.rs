#![cfg(test)]

use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::cisco_verdict_contracts::{
    Configured, FallbackAction, FlaggedAction, InspectionType,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter as _;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use futures_util::stream;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, InspectMessage, Role};
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::hooks::PostCallArgs;
use crate::cisco_stream_scan::{StreamItem, scan_stream};

const FIRST_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#;
const SECOND_CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}]}"#;

fn chunk(json: &str) -> Chunk {
    serde_json::from_str(json).expect("chunk parses")
}

fn chat_chunks() -> Vec<Result<Chunk, io::Error>> {
    vec![Ok(chunk(FIRST_CHUNK)), Ok(chunk(SECOND_CHUNK))]
}

fn passed_chunks() -> Vec<StreamItem> {
    vec![
        StreamItem::Chunk(Box::new(chunk(FIRST_CHUNK))),
        StreamItem::Chunk(Box::new(chunk(SECOND_CHUNK))),
    ]
}

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn guardrail(
    inspection_type: InspectionType,
    fallback_on_error: FallbackAction,
    api_base: String,
) -> CiscoInspectionGuardrail {
    let settings = Configured {
        guardrail_name: "cisco-ai-defense".to_owned(),
        api_key: SecretString::from("key".to_owned()),
        api_base,
        inspection_type,
        inspect_path: "/inspect".to_owned(),
        enabled_rules: Vec::new(),
        on_flagged_action: FlaggedAction::Block,
        fallback_on_error,
        timeout: 10.0,
        integration_profile_id: None,
        integration_profile_version: None,
        integration_tenant_id: None,
        integration_type: None,
    };
    CiscoInspectionGuardrail::new(settings).expect("default TLS backend builds a client")
}

async fn serve(status: StatusCode, body: Vec<u8>) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/inspect",
        post(move || {
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

fn args() -> PostCallArgs<'static> {
    PostCallArgs {
        should_run: true,
        key_user_id: None,
    }
}

fn applied_guardrails(request_data: &Fields) -> Vec<String> {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    match Json::parse(request_data.get(bucket_key).map_or("null", |raw| raw.get())) {
        Json::Object(bucket) => match Json::parse(
            bucket
                .get("applied_guardrails")
                .map_or("null", |raw| raw.get()),
        ) {
            Json::List(items) => items
                .iter()
                .filter_map(|item| match Json::parse(item.get()) {
                    Json::Text(name) => Some(name),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => None,
                })
                .collect(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Vec::new()
            }
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Vec::new(),
    }
}

fn unsupported_event() -> String {
    "data: {\"error\":{\"message\":\"Cisco AI Defense: unsupported streaming format — response withheld for safety\",\"type\":\"guardrail_unsupported_stream\",\"code\":400,\"guardrail\":\"cisco-ai-defense\"}}\n\n".to_owned()
}

fn scan_error_event() -> String {
    "data: {\"error\":{\"message\":\"Cisco AI Defense streaming scan failed — response withheld.\",\"type\":\"guardrail_scan_error\",\"code\":500,\"guardrail\":\"cisco-ai-defense\"}}\n\n".to_owned()
}

fn blocked_event() -> String {
    "data: {\"error\":{\"error\":\"Blocked by Cisco AI Defense Guardrail\",\"message\":\"Blocked by Cisco AI Defense Guardrail\",\"provider\":\"cisco_ai_defense\",\"guardrail\":\"cisco-ai-defense\",\"surface\":\"chat\",\"direction\":\"output\",\"action\":\"block\",\"classifications\":[],\"severity\":null,\"rules\":[],\"explanation\":null,\"event_id\":null,\"code\":400}}\n\n".to_owned()
}

fn fail_closed_event() -> String {
    "data: {\"error\":{\"error\":\"Cisco AI Defense guardrail unavailable\",\"message\":\"Cisco AI Defense scanning service is temporarily unavailable and fallback_on_error='block'\",\"error_type\":\"CiscoAIDefenseGuardrailAPIError\",\"code\":503,\"guardrail\":\"cisco-ai-defense\"}}\n\n".to_owned()
}

#[tokio::test]
async fn passthrough_for_the_mcp_inspection_type() {
    let guardrail = guardrail(
        InspectionType::Mcp,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, passed_chunks());
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn passthrough_when_the_gate_says_no() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let gated_off = PostCallArgs {
        should_run: false,
        key_user_id: None,
    };

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        gated_off,
    )
    .await
    .expect("stream");

    assert_eq!(items, passed_chunks());
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn an_upstream_error_is_returned() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let failing: [Result<Chunk, io::Error>; 2] = [
        Ok(chunk(FIRST_CHUNK)),
        Err(io::Error::other("upstream failed")),
    ];

    let result = scan_stream(&guardrail, &mut request_data, stream::iter(failing), args()).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn an_empty_stream_yields_nothing() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let empty = Vec::<Result<Chunk, io::Error>>::new();

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(empty), args())
        .await
        .expect("stream");

    assert!(items.is_empty());
}

#[tokio::test]
async fn an_unsupported_first_chunk_yields_the_unsupported_event() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let text_chunk = chunk(
        r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"gpt-3.5-turbo-instruct","choices":[{"index":0,"text":"hi","finish_reason":null}]}"#,
    );
    let chunks: [Result<Chunk, io::Error>; 1] = [Ok(text_chunk)];

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(chunks), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(unsupported_event())]);
}

#[tokio::test]
async fn a_text_assembly_yields_the_unsupported_event() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let leading =
        chunk(r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"m","choices":[]}"#);
    let text_chunk = chunk(
        r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"m","choices":[{"index":0,"text":"hi","finish_reason":"stop"}]}"#,
    );
    let chunks: [Result<Chunk, io::Error>; 2] = [Ok(leading), Ok(text_chunk)];

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(chunks), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(unsupported_event())]);
}

#[tokio::test]
async fn an_assembly_error_yields_the_scan_error_event() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let missing_object = chunk(
        r#"{"id":"chatcmpl-1","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":"stop"}]}"#,
    );
    let chunks: [Result<Chunk, io::Error>; 1] = [Ok(missing_object)];

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(chunks), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(scan_error_event())]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[test]
fn the_scan_error_event_carries_the_withheld_message() {
    let items = super::events::event("cisco-ai-defense", super::events::SCAN_ERROR);
    assert_eq!(items, vec![StreamItem::Sse(scan_error_event())]);
}

#[tokio::test]
async fn an_allowed_scan_passes_the_chunks_and_records_the_guardrail() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"allow"}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, passed_chunks());
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_block_yields_the_block_event_and_does_not_record() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(blocked_event())]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn a_fail_closed_api_error_yields_the_error_event() {
    let (base, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(fail_closed_event())]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn a_fail_open_api_error_passes_the_chunks_and_records() {
    let (base, _server) = serve(StatusCode::INTERNAL_SERVER_ERROR, Vec::new()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Allow, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, passed_chunks());
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_redaction_rebuilds_the_stream() {
    let (base, _server) = serve(
        StatusCode::OK,
        br#"{"action":"sanitize","sanitized_text":"[REDACTED]"}"#.to_vec(),
    )
    .await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &guardrail,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items.len(), 1);
    let StreamItem::Chunk(assembled) = &items[0] else {
        panic!("expected a rebuilt chunk, got: {items:?}");
    };
    assert_eq!(assembled.object.as_deref(), Some("chat.completion.chunk"));
    assert_eq!(assembled.id.as_deref(), Some("chatcmpl-1"));
    assert!(assembled.usage.is_some());
    let delta = assembled.choices[0].delta.as_ref().expect("delta");
    assert_eq!(delta.content.as_deref(), Some("[REDACTED]"));
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[test]
fn the_chunk_scan_text_joins_reasoning_with_content_and_arguments_after() {
    let chunks = vec![
        chunk(r#"{"choices":[{"index":0,"delta":{"content":"hi","reasoning_content":"think"}}]}"#),
        chunk(
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}],"function_call":{"name":"g","arguments":"{\"b\":2}"}}}]}"#,
        ),
    ];

    assert_eq!(
        super::scan_text::chunk_scan_text(&chunks),
        "hithink {\"a\":1}{\"b\":2}"
    );
}

#[test]
fn a_scan_text_inside_the_assembled_text_adds_no_message() {
    let response = obj(
        r#"{"choices":[{"index":0,"message":{"content":"hello world","role":"assistant"},"finish_reason":"stop"}]}"#,
    );
    let chunks = vec![chunk(
        r#"{"choices":[{"index":0,"delta":{"content":"hello"}}]}"#,
    )];

    let (messages, scan_text) = super::conversation_messages(&response, &chunks);

    assert_eq!(scan_text, "hello");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello world");
}

#[test]
fn a_scan_text_absent_from_the_assembled_text_is_added_as_a_message() {
    let response = obj(
        r#"{"choices":[{"index":0,"message":{"content":"bye","role":"assistant"},"finish_reason":"stop"}]}"#,
    );
    let chunks = vec![chunk(
        r#"{"choices":[{"index":0,"delta":{"content":"hello"}}]}"#,
    )];

    let (messages, scan_text) = super::conversation_messages(&response, &chunks);

    assert_eq!(scan_text, "hello");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].content, "hello");
    assert!(matches!(messages[1].role, Role::Assistant));
}

#[tokio::test]
async fn an_unserializable_response_yields_the_scan_error_event() {
    let guardrail = guardrail(
        InspectionType::Chat,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj(r#"{"metadata":{}}"#);
    let chunks = vec![chunk(FIRST_CHUNK)];
    let mut response = std::collections::BTreeMap::new();
    response.insert(vec![1_u8, 2, 3], "value");

    let items =
        super::scan_assembled(&guardrail, &mut request_data, &chunks, &response, None).await;

    assert_eq!(items, vec![StreamItem::Sse(scan_error_event())]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[test]
fn the_assembled_contents_are_joined_with_one_space() {
    let messages = vec![
        InspectMessage {
            role: Role::Assistant,
            content: "first".to_owned(),
        },
        InspectMessage {
            role: Role::Assistant,
            content: "second".to_owned(),
        },
    ];

    assert_eq!(super::delivery::joined_contents(&messages), "first second");
}

#[tokio::test]
async fn the_scan_counter_counts_zero_tokens() {
    let counter = super::assembly::ZeroCounter;

    assert_eq!(
        counter
            .count_response_text("gpt-4o", "text")
            .await
            .expect("count"),
        0
    );
    assert_eq!(counter.count_text("text").await.expect("count"), 0);
    assert_eq!(
        counter.count_messages("gpt-4o", None).await.expect("count"),
        0
    );
}

#[tokio::test]
async fn a_stream_over_the_item_limit_is_withheld_when_scanned() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"allow"}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);
    let one = chunk(SECOND_CHUNK);
    let endless =
        (0..=ApiLimits::default().max_items).map(move |_| Ok::<Chunk, io::Error>(one.clone()));

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(endless), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(scan_error_event())]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn a_stream_over_the_item_limit_is_withheld_when_passed_through() {
    let guardrail = guardrail(
        InspectionType::Mcp,
        FallbackAction::Block,
        "http://127.0.0.1:1".to_owned(),
    );
    let mut request_data = obj("{}");
    let one = chunk(SECOND_CHUNK);
    let endless =
        (0..=ApiLimits::default().max_items).map(move |_| Ok::<Chunk, io::Error>(one.clone()));

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(endless), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Sse(scan_error_event())]);
}

#[tokio::test]
async fn monitor_mode_delivers_a_flagged_stream_and_records() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let mut monitoring = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    monitoring.settings.on_flagged_action = FlaggedAction::Monitor;
    let mut request_data = obj(r#"{"metadata":{}}"#);

    let items = scan_stream(
        &monitoring,
        &mut request_data,
        stream::iter(chat_chunks()),
        args(),
    )
    .await
    .expect("stream");

    assert_eq!(items, passed_chunks());
    assert_eq!(applied_guardrails(&request_data), vec!["cisco-ai-defense"]);
}

#[tokio::test]
async fn a_response_without_messages_passes_the_chunks_unexamined() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);
    let chunks = vec![chunk(
        r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":"stop"}]}"#,
    )];
    let response = obj(
        r#"{"choices":[{"index":0,"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#,
    );

    let items =
        super::inspect_and_deliver(&guardrail, &mut request_data, &chunks, response, None).await;

    assert_eq!(items, vec![StreamItem::Chunk(Box::new(chunks[0].clone()))]);
    assert!(applied_guardrails(&request_data).is_empty());
}

#[tokio::test]
async fn a_stream_without_content_passes_through_a_blocking_scan() {
    let (base, _server) = serve(StatusCode::OK, br#"{"action":"block"}"#.to_vec()).await;
    let guardrail = guardrail(InspectionType::Chat, FallbackAction::Block, base);
    let mut request_data = obj(r#"{"metadata":{}}"#);
    let role_only = chunk(
        r#"{"id":"c1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":"stop"}]}"#,
    );
    let chunks: [Result<Chunk, io::Error>; 1] = [Ok(role_only.clone())];

    let items = scan_stream(&guardrail, &mut request_data, stream::iter(chunks), args())
        .await
        .expect("stream");

    assert_eq!(items, vec![StreamItem::Chunk(Box::new(role_only))]);
    assert!(applied_guardrails(&request_data).is_empty());
}

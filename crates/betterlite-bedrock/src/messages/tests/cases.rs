use std::collections::BTreeMap;

use betterlite_anthropic::params::output_config::DROP_OUTPUT_CONFIG_WARNING;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::{IndexMap, IndexSet};
use secrecy::SecretString;

use crate::bedrock_settings::ErrorHeaders;
use crate::invoke_primitives::url::{InvokeEnvironment, InvokeModelUrl};
use crate::messages::config::MessagesConfig;
use crate::messages::tests::support::{
    EFFORT_TIER_INFO, Fixture, Keys, Session, Signing, TestFrame, body_json, call, capture_logs,
    close_after_first, contract, declared, header, js, kept_warnings, obj, sign, stream_all,
    test_frame,
};
use crate::messages::transport::{
    MessagesEndpoint, get_error_class, messages_complete_url, validate_messages_environment,
};
use crate::request_policy::{BedrockRequestPolicy, Params as MetadataParams, RequestValue};

const MODEL_SONNET: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";
const STREAM_URL: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-4-5-20250929-v1:0/invoke-with-response-stream";
const INVOKE_BASE: &str = "https://proxy.test/bedrock";
const INVOKE_URL: &str =
    "https://proxy.test/bedrock/model/anthropic.claude-sonnet-4-5-20250929-v1:0/invoke";
const AUTHZ: &str = "AWS4-HMAC-SHA256 Credential=AKIAFIXTURE/20231114/us-west-2/bedrock/aws4_request, SignedHeaders=content-type;host;x-amz-date;x-amzn-bedrock-request-metadata, Signature=1f771a15e6f59a16411903e4e0cee2995810f9b2cd260b2969e4d09101604861";
const METADATA_HEADER: &str = r#"{"user_api_key_hash":"hash-1"}"#;
const REQUEST_BODY: &str = r#"{"messages": [{"role": "user", "content": "hi"}], "max_tokens": 8}"#;
const USER_MESSAGES: &str = r#"[{"role": "user", "content": "hi"}]"#;

const NATIVE_MESSAGES: &str = r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"},{"role":"assistant","content":[{"type":"text","text":"hello"}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}]"#;
const NATIVE_PARAMS: &str = r#"{"max_tokens":1024,"temperature":0.2,"stop_sequences":["END"],"top_p":0.9,"metadata":{"user_id":"u1"},"stream":true,"mystery":5,"anthropic_version":"custom-version"}"#;
const NATIVE_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":[{"type":"text","text":"hello"}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}],"max_tokens":1024,"temperature":0.2,"stop_sequences":["END"],"top_p":0.9,"metadata":{"user_id":"u1"},"anthropic_version":"custom-version","system":[{"type":"text","text":"be nice"}]}"#;

const BETA_PARAMS: &str = r#"{"max_tokens":512,"tools":[{"type":"tool_search_tool_regex_20251119","name":"tool_search"},{"name":"geo","description":"geo","input_schema":{"type":"object","properties":{"q":{"type":"string"}}},"eager_input_streaming":true}],"safeguards":[]}"#;

const ADAPTIVE_PARAMS: &str =
    r#"{"max_tokens":1024,"thinking":{"type":"adaptive","display":"updates"}}"#;
const ADAPTIVE_PLAIN: &str = r#"{"max_tokens":1024,"thinking":{"type":"adaptive"}}"#;
const ADAPTIVE_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":1024,"thinking":{"type":"adaptive","display":"updates"},"anthropic_version":"bedrock-2023-05-31","anthropic_beta":["thinking-display-updates-2026-08-18"]}"#;
const THINKING_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":1024,"thinking":{"type":"adaptive"},"anthropic_version":"bedrock-2023-05-31"}"#;

const DROP_PLAIN_PARAMS: &str = r#"{"max_tokens":256,"output_config":{"effort":"high"}}"#;
const DROP_TIER_PARAMS: &str = r#"{"max_tokens":256,"output_config":{"effort":"xhigh"}}"#;
const DROP_PLAIN_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":256,"anthropic_version":"bedrock-2023-05-31"}"#;
const DROP_TIER_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":256,"output_config":{"effort":"high"},"anthropic_version":"bedrock-2023-05-31","anthropic_beta":["effort-2025-11-24"]}"#;
const DROP_WARNING: &str = "Dropping adaptive `thinking`/`output_config.effort` for model=fixture-plain: the model does not support extended thinking, or max_tokens is too small to fit the minimum thinking budget.";

const ORDER_PARAMS: &str = r#"{"max_tokens":256,"tools":[{"name":"geo","input_schema":{"type":"object"},"eager_input_streaming":true,"custom":{"defer_loading":true}}],"safeguards":[],"mcp_servers":[{"type":"url","url":"https://example.test/mcp"}]}"#;
const ORDER_BODY: &str = r#"{"messages":[{"role":"user","content":"hi"}],"max_tokens":256,"tools":[{"name":"geo","input_schema":{"type":"object"},"defer_loading":true}],"safeguards":[],"anthropic_version":"bedrock-2023-05-31","anthropic_beta":["dangerous-tool-use-2026-09-03","fine-grained-tool-streaming-2025-05-14"]}"#;

const WEB_SEARCH_PARAMS: &str =
    r#"{"max_tokens":256,"tools":[{"type":"web_search_20250305","name":"web_search"}]}"#;
const MID_SYSTEM_MESSAGES: &str = r#"[{"role":"user","content":"hi"},{"role":"system","content":"be nice"},{"role":"user","content":"ok"}]"#;
const MID_SYSTEM_PARAMS: &str = r#"{"max_tokens":64}"#;
const STRUCTURED_PARAMS: &str = r#"{"max_tokens":256,"output_config":{"format":{"type":"json_schema","schema":{"type":"object","properties":{"a":{"type":"string"}}}}}}"#;
const CLEAR_THINKING_PARAMS: &str = r#"{"max_tokens":1024,"thinking":{"type":"enabled","budget_tokens":1024},"context_management":{"edits":[{"type":"clear_thinking_20251015"}]}}"#;

const EVENT_START: &str = r#"{"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "content": [], "model": "claude", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 11, "output_tokens": 1, "cache_read_input_tokens": 4}}}"#;
const EVENT_CONTENT: &str = r#"{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello"}}"#;
const EVENT_DELTA: &str = r#"{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 7}}"#;
const EVENT_STOP: &str =
    r#"{"type": "message_stop", "usage": {"input_tokens": 11, "cache_creation_input_tokens": 3}}"#;

const SSE_START: &str = r#"event: message_start
data: {"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "content": [], "model": "claude", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 11, "output_tokens": 1, "cache_read_input_tokens": 4}}}

"#;
const SSE_CONTENT: &str = r#"event: content_block_delta
data: {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello"}}

"#;
const SSE_DELTA: &str = r#"event: message_delta
data: {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 7, "cache_creation_input_tokens": 3, "input_tokens": 11, "cache_read_input_tokens": 4}}

"#;
const SSE_STOP: &str = r#"event: message_stop
data: {"type": "message_stop", "usage": {"input_tokens": 11, "cache_creation_input_tokens": 3}}

"#;

fn stream_headers() -> Option<BTreeMap<String, String>> {
    let mut headers = BTreeMap::new();
    headers.insert(
        "content-type".to_owned(),
        "application/vnd.amazon.eventstream".to_owned(),
    );
    Some(headers)
}

fn metadata_params() -> MetadataParams {
    let mut entries: IndexMap<String, RequestValue> = IndexMap::new();
    entries.insert(
        "user_api_key_hash".to_owned(),
        RequestValue::Text("hash-1".to_owned()),
    );
    entries.insert("team".to_owned(), RequestValue::Text("core".to_owned()));
    let mut params = MetadataParams::new();
    params.insert("metadata".to_owned(), RequestValue::Mapping(entries));
    params
}

pub(crate) fn messages_invoke_url() {
    let merged = merged_environment();
    let streamed = endpoint_url(
        None,
        Some(true),
        InvokeEnvironment {
            region_name: Some("us-east-1"),
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
    );
    assert_eq!(streamed.endpoint_url, STREAM_URL);
    signed_stream_request(&merged, &streamed);
    proxy_variant();
}

fn endpoint_url(
    api_base: Option<&str>,
    stream: Option<bool>,
    environment: InvokeEnvironment<'_>,
) -> InvokeModelUrl {
    messages_complete_url(MessagesEndpoint {
        api_base,
        model: MODEL_SONNET,
        optional_params: &Default::default(),
        stream,
        environment,
    })
    .expect("the endpoint url builds")
}

fn merged_environment() -> IndexMap<String, String> {
    let mut input: IndexMap<String, String> = IndexMap::new();
    input.insert("x-custom".to_owned(), "keep".to_owned());
    input.insert(
        "X-Amzn-Bedrock-Request-Metadata".to_owned(),
        "forged".to_owned(),
    );
    let fields: IndexSet<String> = ["user_api_key_hash".to_owned()].into_iter().collect();
    let policy = BedrockRequestPolicy { fields };
    let (merged, api_base) =
        validate_messages_environment(&input, &metadata_params(), Some(&policy), None, None)
            .expect("the environment merges");
    assert_eq!(api_base, None);
    assert_eq!(merged.get("x-custom").map(String::as_str), Some("keep"));
    assert_eq!(
        merged
            .get("X-Amzn-Bedrock-Request-Metadata")
            .map(String::as_str),
        Some(METADATA_HEADER)
    );
    assert_eq!(merged.len(), 2);
    merged
}

fn signed_stream_request(merged: &IndexMap<String, String>, url: &InvokeModelUrl) {
    let signed = sign(
        Signing {
            model: MODEL_SONNET,
            headers: merged,
            request_data: &contract(REQUEST_BODY),
            api_base: &url.endpoint_url,
            api_key: None,
            configured_bearer: None,
        },
        &Keys {
            access_key_id: "AKIAFIXTURE",
            secret_access_key: "fixture-secret",
            session_token: None,
        },
    )
    .expect("the request signs");
    assert_eq!(signed.method, "POST");
    assert_eq!(signed.url, STREAM_URL);
    assert_eq!(header(&signed.headers, "Authorization"), Some(AUTHZ));
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(
        header(&signed.headers, "X-Amz-Date"),
        Some("20231114T221320Z")
    );
    assert_eq!(
        header(&signed.headers, "X-Amzn-Bedrock-Request-Metadata"),
        Some(METADATA_HEADER)
    );
    assert_eq!(header(&signed.headers, "x-custom"), Some("keep"));
    assert_eq!(signed.headers.len(), 5);
    assert_eq!(signed.body, REQUEST_BODY.as_bytes());
}

fn proxy_variant() {
    let api_key = SecretString::from("fixture-api-key");
    let merged = proxy_environment(&api_key);
    let url = endpoint_url(
        Some(INVOKE_BASE),
        None,
        InvokeEnvironment {
            region_name: None,
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
    );
    assert_eq!(url.endpoint_url, INVOKE_URL);
    proxy_signed_request(&merged, &url, &api_key);
}

fn proxy_environment(api_key: &SecretString) -> IndexMap<String, String> {
    let empty: IndexMap<String, String> = IndexMap::new();
    let (merged, api_base) = validate_messages_environment(
        &empty,
        &MetadataParams::new(),
        None,
        Some(INVOKE_BASE),
        Some(api_key),
    )
    .expect("the environment passes the base through");
    assert!(merged.is_empty());
    assert_eq!(api_base.as_deref(), Some(INVOKE_BASE));
    merged
}

fn proxy_signed_request(
    merged: &IndexMap<String, String>,
    url: &InvokeModelUrl,
    api_key: &SecretString,
) {
    let signed = sign(
        Signing {
            model: MODEL_SONNET,
            headers: merged,
            request_data: &contract("{}"),
            api_base: &url.endpoint_url,
            api_key: Some(api_key),
            configured_bearer: None,
        },
        &Keys {
            access_key_id: "AKIAFIXTURE",
            secret_access_key: "fixture-secret",
            session_token: Some("fixture-session"),
        },
    )
    .expect("the proxy request signs");
    assert_eq!(
        header(&signed.headers, "Authorization"),
        Some("Bearer fixture-api-key")
    );
    assert_eq!(
        header(&signed.headers, "Content-Type"),
        Some("application/json")
    );
    assert_eq!(signed.headers.len(), 2);
    assert_eq!(signed.body, b"{}".to_vec());
}

pub(crate) fn messages_native_body() {
    let (_guard, logs) = capture_logs();
    let body = declared(
        &call("fixture-plain", NATIVE_MESSAGES, NATIVE_PARAMS, "{}"),
        false,
    )
    .expect("the native request transforms");
    assert_eq!(body_json(&body), js(NATIVE_BODY));
    assert_eq!(kept_warnings(&logs), Vec::<String>::new());
}

pub(crate) fn anthropic_beta_body() {
    let body = declared(
        &call(
            "fixture-tool-search",
            USER_MESSAGES,
            BETA_PARAMS,
            r#"{"anthropic-beta":"advanced-tool-use-2025-11-20"}"#,
        ),
        false,
    )
    .expect("the beta request transforms");
    assert_eq!(
        body.get("anthropic_beta"),
        Some(&js(
            r#"["dangerous-tool-use-2026-09-03","fine-grained-tool-streaming-2025-05-14","tool-search-tool-2025-10-19"]"#
        ))
    );
    assert!(!body.contains_key("stream"));
    assert!(!body.contains_key("anthropic-beta"));
}

pub(crate) fn adaptive_display_updates() {
    let fixture = Fixture::new(EFFORT_TIER_INFO);
    let mut session = Session::new(&fixture);
    let (_guard, logs) = capture_logs();
    let first = session
        .transform(
            &call("fixture-adaptive", USER_MESSAGES, ADAPTIVE_PARAMS, "{}"),
            false,
        )
        .expect("the display request transforms");
    let second = session
        .transform(
            &call("fixture-adaptive", USER_MESSAGES, ADAPTIVE_PLAIN, "{}"),
            false,
        )
        .expect("the plain adaptive request transforms");
    assert_eq!(body_json(&first), js(ADAPTIVE_BODY));
    assert_eq!(body_json(&second), js(THINKING_BODY));
    assert_eq!(kept_warnings(&logs), Vec::<String>::new());
    clear_thinking(&mut session);
}

// The clear_thinking edit makes the Bedrock config inject adaptive thinking before the strip pass.
fn clear_thinking(session: &mut Session<'_>) {
    let (_guard, logs) = capture_logs();
    let body = session
        .transform(
            &call(
                "fixture-adaptive",
                USER_MESSAGES,
                CLEAR_THINKING_PARAMS,
                "{}",
            ),
            false,
        )
        .expect("the clear-thinking request transforms");
    assert!(matches!(
        body.get("thinking"),
        Some(Json::Object(fields))
            if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "adaptive")
    ));
    assert!(!body.contains_key("output_config"));
    assert!(
        kept_warnings(&logs)
            .iter()
            .any(|message| message.contains("stripping unsupported `output_config`"))
    );
}

pub(crate) fn unsupported_output_config_warn_drop() {
    transform_drop();
    direct_drop();
    pipeline_drop_output_config();
}

fn transform_drop() {
    let fixture = Fixture::new(EFFORT_TIER_INFO);
    let mut session = Session::new(&fixture);
    let (_guard, logs) = capture_logs();
    let plain = session
        .transform(
            &call("fixture-plain", USER_MESSAGES, DROP_PLAIN_PARAMS, "{}"),
            true,
        )
        .expect("the plain effort request transforms");
    let tier = session
        .transform(
            &call("fixture-effort-tier", USER_MESSAGES, DROP_TIER_PARAMS, "{}"),
            true,
        )
        .expect("the tiered effort request transforms");
    assert_eq!(body_json(&plain), js(DROP_PLAIN_BODY));
    assert_eq!(body_json(&tier), js(DROP_TIER_BODY));
    assert_eq!(kept_warnings(&logs), vec![DROP_WARNING.to_owned()]);
}

fn direct_drop() {
    let fixture = Fixture::new(EFFORT_TIER_INFO);
    let mut session = Session::new(&fixture);
    let (_guard, logs) = capture_logs();
    let mut body = obj(r#"{"output_config":{"format":{"type":"json_schema"},"effort":"high"}}"#);
    session.drop_output_config(&mut body, "fixture-plain", true);
    assert!(!body.contains_key("output_config"));
    assert_eq!(
        kept_warnings(&logs),
        vec![DROP_OUTPUT_CONFIG_WARNING.replacen("%s", "fixture-plain", 1)]
    );

    let mut format_only = obj(r#"{"output_config":{"format":{"type":"json_schema"}}}"#);
    session.drop_output_config(&mut format_only, "fixture-plain", true);
    assert!(format_only.contains_key("output_config"));
    let mut supported = obj(r#"{"output_config":{"effort":"high"}}"#);
    session.drop_output_config(&mut supported, "fixture-effort-tier", true);
    assert!(supported.contains_key("output_config"));
    let mut kept = obj(r#"{"output_config":{"format":{"type":"json_schema"}}}"#);
    session.drop_output_config(&mut kept, "fixture-plain", false);
    assert!(kept.contains_key("output_config"));

    // The final drop runs inside the transform when the model takes output_config but not effort.
    let body = session
        .transform(
            &call(
                "fixture-output-config",
                USER_MESSAGES,
                r#"{"max_tokens":256,"output_config":{"effort":"high"}}"#,
                "{}",
            ),
            true,
        )
        .expect("the output-config request transforms");
    assert!(!body.contains_key("output_config"));
    assert!(
        kept_warnings(&logs).iter().any(|message| message
            == &DROP_OUTPUT_CONFIG_WARNING.replacen("%s", "fixture-output-config", 1))
    );
}

pub(crate) fn normalization_order() {
    let (_guard, logs) = capture_logs();
    let body = declared(
        &call("fixture-plain", USER_MESSAGES, ORDER_PARAMS, "{}"),
        false,
    )
    .expect("the normalization request transforms");
    assert_eq!(body_json(&body), js(ORDER_BODY));
    assert_eq!(kept_warnings(&logs), Vec::<String>::new());
    structured_output();
    mid_conversation_system();
}

fn structured_output() {
    let (_guard, logs) = capture_logs();
    let unsupported = declared(
        &call("fixture-plain", USER_MESSAGES, STRUCTURED_PARAMS, "{}"),
        false,
    )
    .expect("the unsupported structured request transforms");
    assert!(!unsupported.contains_key("output_config"));
    assert!(
        kept_warnings(&logs)
            .iter()
            .any(|message| message.contains("does not advertise"))
    );
    let supported = declared(
        &call("fixture-structured", USER_MESSAGES, STRUCTURED_PARAMS, "{}"),
        false,
    )
    .expect("the supported structured request transforms");
    let Some(Json::Object(output_config)) = supported.get("output_config") else {
        panic!("the supported model keeps output_config");
    };
    assert!(output_config.contains_key("format"));
}

fn mid_conversation_system() {
    assert!(!kept_system_turn("fixture-plain"));
    assert!(kept_system_turn("fixture-mid-system"));
}

fn kept_system_turn(model: &str) -> bool {
    let body = declared(
        &call(model, MID_SYSTEM_MESSAGES, MID_SYSTEM_PARAMS, "{}"),
        false,
    )
    .expect("the mid-conversation request transforms");
    let Some(Json::List(messages)) = body.get("messages") else {
        panic!("the body carries messages");
    };
    messages.iter().skip(1).any(|message| {
        matches!(
            message,
            Json::Object(fields)
                if matches!(fields.get("role"), Some(Json::Text(role)) if role == "system")
        )
    })
}

pub(crate) fn error_header_projection() {
    let headers = vec![("x-amzn-requestid".to_owned(), "req-1".to_owned())];
    let error = get_error_class(
        r#"validationException {"message": "bad input"}"#,
        400,
        &headers,
    )
    .expect("the error builds");
    assert_eq!(error.error.status_code, 400);
    assert_eq!(
        error.error.message,
        r#"validationException {"message": "bad input"}"#
    );
    match error.headers {
        Some(ErrorHeaders::Http(projected)) => assert_eq!(projected, headers),
        Some(ErrorHeaders::Mapping(_)) | None => {
            panic!("the supplied headers project as HTTP pairs");
        }
    }
    let rejected = declared(
        &call("fixture-plain", USER_MESSAGES, WEB_SEARCH_PARAMS, "{}"),
        false,
    )
    .expect_err("the web search tool is rejected");
    assert_eq!(rejected.status_code(), Some(400));
}

pub(crate) fn messages_constructor() {
    assert!(MessagesConfig::try_from(&Object::new()).is_ok());
    let mut kwargs = Object::new();
    kwargs.insert("model".to_owned(), Json::Text("fixture-plain".to_owned()));
    let error = MessagesConfig::try_from(&kwargs).expect_err("nonempty kwargs fail");
    assert_eq!(
        error.to_string(),
        "object.__init__() takes exactly one argument (the instance to initialize)"
    );
}

pub(crate) fn aws_event_to_sse() {
    let mut frame = Vec::new();
    frame.extend(test_frame(TestFrame::Chunk(EVENT_START)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_CONTENT)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_DELTA)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_STOP)));
    let (emitted, failure) = stream_all(stream_headers(), vec![frame]);
    assert!(failure.is_none());
    let expected = [SSE_START, SSE_CONTENT, SSE_DELTA, SSE_STOP];
    assert_eq!(emitted.len(), expected.len());
    for (piece, want) in emitted.iter().zip(expected) {
        assert_eq!(String::from_utf8(piece.clone()).expect("SSE text"), want);
    }
}

pub(crate) fn message_start_event() {
    let mut frame = Vec::new();
    frame.extend(test_frame(TestFrame::Chunk(EVENT_START)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_STOP)));
    let (emitted, failure) = stream_all(stream_headers(), vec![frame]);
    assert!(failure.is_none());
    assert_eq!(emitted.len(), 2);
    assert_eq!(
        String::from_utf8(emitted[0].clone()).expect("SSE text"),
        SSE_START
    );
    assert_eq!(
        String::from_utf8(emitted[1].clone()).expect("SSE text"),
        SSE_STOP
    );
    unfinished_stream();
    closed_stream();
}

// A body cut before message_stop still yields the buffered delta when the iterator finishes.
fn unfinished_stream() {
    let mut frame = Vec::new();
    frame.extend(test_frame(TestFrame::Chunk(EVENT_START)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_DELTA)));
    let (emitted, failure) = stream_all(stream_headers(), vec![frame]);
    assert!(failure.is_none());
    assert_eq!(emitted.len(), 2);
    assert_eq!(
        String::from_utf8(emitted[0].clone()).expect("SSE text"),
        SSE_START
    );
    assert!(
        String::from_utf8(emitted[1].clone())
            .expect("SSE text")
            .starts_with("event: message_delta\n")
    );
}

fn closed_stream() {
    let mut frame = Vec::new();
    frame.extend(test_frame(TestFrame::Chunk(EVENT_START)));
    frame.extend(test_frame(TestFrame::Chunk(EVENT_STOP)));
    let closure = close_after_first(vec![frame]);
    assert_eq!(closure.first, Some(SSE_START.as_bytes().to_vec()));
    assert!(closure.closed);
    assert_eq!(closure.after, Ok(None));
}

pub(crate) fn native_error_event() {
    let frame = test_frame(TestFrame::Failure(
        "validationException",
        br#"{"message": "bad input"}"#,
    ));
    let (emitted, failure) = stream_all(stream_headers(), vec![frame]);
    assert!(emitted.is_empty());
    let error = failure.expect("the error frame fails the stream");
    assert_eq!(error.kind, "BedrockError");
    assert_eq!(
        error.message,
        r#"validationException {"message": "bad input"}"#
    );
    assert_eq!(error.status, Some(400));
}

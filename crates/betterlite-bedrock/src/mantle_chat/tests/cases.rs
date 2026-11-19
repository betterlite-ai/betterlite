use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::api::messages_errors::fields::Fields;
use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json as ParamJson;
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use indexmap::{IndexMap, IndexSet};
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use crate::invoke_events::{DecoderClock, InvokeChunk};
use crate::mantle_chat::HAS_CUSTOM_STREAM_WRAPPER;
use crate::mantle_chat::environment::{chat_headers, messages_headers};
use crate::mantle_chat::messages::{move_betas_into_header, restore_messages_fields};
use crate::mantle_chat::request::restore_body_fields;
use crate::mantle_chat::response::transform_response;
use crate::mantle_chat::stream::{messages_iterator, model_response_iterator};
use crate::mantle_chat::url::complete_url;
use crate::mantle_inputs::records::MantleGlobals;
use crate::request_policy::{BedrockRequestPolicy, Params as MetadataParams, RequestValue};

use super::support::{
    ChatSession, Fixture, MapFetcher, MessagesSession, Mode, Recorder, contract_of, invoke_params,
    js, logging_object, map_of, messages_call, messages_of, obj, recorder_deps, runtime,
};

pub(super) const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

const USER_MESSAGES: &str = r#"[{"role": "user", "content": "hi"}]"#;
const BETA_PARAMS: &str = r#"{"max_tokens":512,"tools":[{"type":"tool_search_tool_regex_20251119","name":"tool_search"},{"name":"geo","description":"geo","input_schema":{"type":"object","properties":{"q":{"type":"string"}}},"eager_input_streaming":true}],"safeguards":[]}"#;
const BETA_HEADER: &str = "dangerous-tool-use-2026-09-03,fine-grained-tool-streaming-2025-05-14,tool-examples-2025-10-29,tool-search-tool-2025-10-19";
pub(super) const IMAGE_BODY: &str =
    r#"{"data":"iVBORw0KGgo=","media_type":"image/png","type":"base64"}"#;
const STRUCTURED_PARAMS: &str = r#"{"max_tokens":256,"output_config":{"format":{"type":"json_schema","schema":{"type":"object","properties":{"a":{"type":"string"}}}}}}"#;

pub(super) fn url(
    api_base: Option<&str>,
    model: &str,
    params: &str,
    globals: &MantleGlobals,
) -> String {
    complete_url(api_base, model, &invoke_params(params), globals).expect("the mantle url resolves")
}

fn base_globals(base: &str) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: None,
        mantle_api_base: Some(base.to_owned()),
        mantle_region: None,
        aws_bearer_token: None,
        aws_region_name: None,
        aws_region: None,
    }
}

fn policy_of(field: &str) -> BedrockRequestPolicy {
    BedrockRequestPolicy {
        fields: IndexSet::from([field.to_owned()]),
    }
}

fn metadata_params(name: &str) -> MetadataParams {
    let mut metadata = MetadataParams::new();
    metadata.insert(
        "user_api_key_alias".to_owned(),
        RequestValue::Text(name.to_owned()),
    );
    let mut params = MetadataParams::new();
    params.insert("metadata".to_owned(), RequestValue::Mapping(metadata));
    params.insert(
        "aws_bedrock_project_id".to_owned(),
        RequestValue::Text(name.to_owned()),
    );
    params
}

fn prompt_messages() -> Vec<ParamJson> {
    messages_of(r#"[{"role":"user","content":"hi"}]"#)
}

pub(super) fn mantle_chat_url() {
    url_cases();
    url_error_cases();
    chat_environment_cases();
}

type UrlRow = (
    Option<&'static str>,
    &'static str,
    &'static str,
    &'static str,
);

// Both configs share B591/B690, so one table proves the region and endpoint rules for the chat surface too.
const URL_ROWS: &[UrlRow] = &[
    (
        None,
        "claude",
        "{}",
        "https://bedrock-mantle.us-west-2.api.aws",
    ),
    (
        None,
        "claude",
        r#"{"aws_region_name":"eu-west-1"}"#,
        "https://bedrock-mantle.eu-west-1.api.aws",
    ),
    (
        None,
        "arn:aws:bedrock:ap-south-1:123:inference-profile/x",
        "{}",
        "https://bedrock-mantle.ap-south-1.api.aws",
    ),
    (
        None,
        "claude",
        r#"{"aws_bedrock_runtime_endpoint":"https://runtime.test/"}"#,
        "https://runtime.test",
    ),
    (
        Some("https://proxy.test/"),
        "claude",
        "{}",
        "https://proxy.test",
    ),
];

fn url_cases() {
    url_rows(&MantleGlobals::default(), URL_ROWS);
    url_rows(
        &base_globals("https://mantle.test/v1"),
        &[(None, "claude", "{}", "https://mantle.test")],
    );
}

fn url_rows(globals: &MantleGlobals, rows: &[UrlRow]) {
    for (api_base, model, params, base) in rows {
        let expected = format!("{base}/anthropic/v1/messages");
        assert_eq!(url(*api_base, model, params, globals), expected);
    }
}

fn url_error_cases() {
    let globals = MantleGlobals::default();
    let rows = [
        (
            r#"{"aws_region_name":"US WEST"}"#,
            "invalid AWS region format: US WEST",
        ),
        (
            r#"{"aws_region_name":5}"#,
            "field aws_region_name is not a string",
        ),
    ];
    for (params, message) in rows {
        let error = complete_url(None, "claude", &invoke_params(params), &globals)
            .expect_err("the region setting is invalid");
        assert_eq!(error.to_string(), message);
    }
}

fn chat_environment_cases() {
    let policy = policy_of("user_api_key_alias");
    let merged = chat_guardrail_headers(&policy);
    merged_guardrail_headers(&merged);
    empty_project_id_keeps_header(&policy);
}

fn merged_guardrail_headers(merged: &IndexMap<String, String>) {
    assert_eq!(merged.get("x-custom").map(String::as_str), Some("keep"));
    assert_eq!(
        merged
            .get("X-Amzn-Bedrock-GuardrailIdentifier")
            .map(String::as_str),
        Some("g-1")
    );
    assert_eq!(
        merged
            .get("X-Amzn-Bedrock-GuardrailVersion")
            .map(String::as_str),
        Some("2")
    );
    assert_eq!(
        merged.get("X-Amzn-Bedrock-Trace").map(String::as_str),
        Some("ENABLED_FULL")
    );
    assert_eq!(
        merged
            .get("X-Amzn-Bedrock-Request-Metadata")
            .map(String::as_str),
        Some(r#"{"user_api_key_alias":"team-a"}"#)
    );
    assert_eq!(
        merged.get("anthropic-workspace").map(String::as_str),
        Some("team-a")
    );
    assert!(!merged.contains_key("authorization"));
}

fn empty_project_id_keeps_header(policy: &BedrockRequestPolicy) {
    let mut headers = IndexMap::new();
    headers.insert("anthropic-workspace".to_owned(), "old".to_owned());
    let mut empty = invoke_params("{}");
    let merged = chat_headers(&headers, &mut empty, &metadata_params(""), Some(policy))
        .expect("the empty project id merges");
    assert_eq!(
        merged.get("anthropic-workspace").map(String::as_str),
        Some("old")
    );
}

fn chat_guardrail_headers(policy: &BedrockRequestPolicy) -> IndexMap<String, String> {
    let mut params = invoke_params(
        r#"{"guardrailConfig":{"guardrailIdentifier":"g-1","guardrailVersion":"2","trace":"enabled_full"}}"#,
    );
    let mut headers = IndexMap::new();
    headers.insert("x-custom".to_owned(), "keep".to_owned());
    headers.insert("anthropic-workspace".to_owned(), "old".to_owned());
    let merged = chat_headers(
        &headers,
        &mut params,
        &metadata_params("team-a"),
        Some(policy),
    )
    .expect("the chat environment merges");
    assert!(!params.contains_key("guardrailConfig"));
    merged
}

pub(super) fn mantle_message_url() {
    url_cases();
    url_error_cases();
    message_environment_cases();
}

fn message_environment_cases() {
    let empty = IndexMap::new();
    messages_version_default(&empty);
    messages_version_caller();
    messages_workspace_and_metadata(&empty);
}

fn messages_version_default(empty: &IndexMap<String, String>) {
    let (merged, base) = messages_headers(
        empty,
        &MetadataParams::new(),
        None,
        Some("https://proxy.test"),
        None,
    )
    .expect("the messages environment merges");
    assert_eq!(
        merged.get("anthropic-version").map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(base.as_deref(), Some("https://proxy.test"));
}

fn messages_version_caller() {
    let mut versioned = IndexMap::new();
    versioned.insert("Anthropic-Version".to_owned(), String::new());
    let (merged, base) = messages_headers(&versioned, &MetadataParams::new(), None, None, None)
        .expect("the caller version stays");
    assert_eq!(merged.len(), 1);
    assert_eq!(
        merged.get("Anthropic-Version").map(String::as_str),
        Some("")
    );
    assert_eq!(base, None);
}

fn messages_workspace_and_metadata(empty: &IndexMap<String, String>) {
    caller_workspace_kept();
    metadata_and_api_key_headers(empty);
}

fn caller_workspace_kept() {
    let mut caller = IndexMap::new();
    caller.insert("Anthropic-Workspace".to_owned(), "caller".to_owned());
    let (merged, _) = messages_headers(&caller, &metadata_params("team-a"), None, None, None)
        .expect("the workspace joins");
    assert_eq!(
        merged.get("anthropic-workspace").map(String::as_str),
        Some("team-a")
    );
    assert_eq!(
        merged.get("Anthropic-Workspace").map(String::as_str),
        Some("caller")
    );
}

fn metadata_and_api_key_headers(empty: &IndexMap<String, String>) {
    let (merged, _) = messages_headers(
        empty,
        &metadata_params("team-a"),
        Some(&policy_of("user_api_key_alias")),
        None,
        None,
    )
    .expect("the metadata header joins");
    assert_eq!(
        merged
            .get("X-Amzn-Bedrock-Request-Metadata")
            .map(String::as_str),
        Some(r#"{"user_api_key_alias":"team-a"}"#)
    );
    let key = SecretString::from("fixture-api-key");
    let (merged, _) = messages_headers(
        empty,
        &MetadataParams::new(),
        None,
        Some("https://proxy.test"),
        Some(&key),
    )
    .expect("the api key never becomes a header");
    assert!(
        merged
            .values()
            .all(|value| !value.contains("fixture-api-key"))
    );
}

pub(super) fn beta_header_not_body() {
    moved_betas_through_transform();
    beta_provider_is_mantle();
    beta_shapes();
    restored_message_copy_semantics();
}

// The bedrock mapping nulls this token, so it surviving proves the filter ran under bedrock_mantle.
fn beta_provider_is_mantle() {
    let fixture = Fixture::new();
    let mut session = MessagesSession::new(&fixture);
    let mut headers = IndexMap::new();
    headers.insert(
        "anthropic-beta".to_owned(),
        "structured-outputs-2025-11-13".to_owned(),
    );
    let call = messages_call(
        "mantle/fixture-plain",
        USER_MESSAGES,
        r#"{"max_tokens":8}"#,
        r#"{"anthropic-beta":"structured-outputs-2025-11-13"}"#,
    );
    let body = session
        .transform(&call, &mut headers)
        .expect("the user beta survives under the mantle mapping");
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some("structured-outputs-2025-11-13")
    );
    assert!(!body.contains_key("anthropic_beta"));
}

fn moved_betas_through_transform() {
    let fixture = Fixture::new();
    let mut session = MessagesSession::new(&fixture);
    let (headers, body) = beta_round(
        &mut session,
        "mantle/fixture-tool-search",
        BETA_PARAMS,
        r#"{"anthropic-beta":"advanced-tool-use-2025-11-20"}"#,
    );
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some(BETA_HEADER)
    );
    assert_eq!(body.get("model"), Some(&js(r#""fixture-tool-search""#)));
    let (headers, body) = beta_round(
        &mut session,
        "mantle/fixture-plain",
        r#"{"max_tokens":8,"stream":false}"#,
        "{}",
    );
    assert!(!headers.contains_key("anthropic-beta"));
    assert!(!body.contains_key("stream"));
    assert_eq!(body.get("model"), Some(&js(r#""fixture-plain""#)));
}

// A round carries the caller's lower-case and upper-case beta headers into one transform.
fn beta_round(
    session: &mut MessagesSession<'_>,
    model: &str,
    params: &str,
    request_headers: &str,
) -> (IndexMap<String, String>, Object) {
    let mut headers = IndexMap::new();
    headers.insert("anthropic-beta".to_owned(), "caller-beta".to_owned());
    headers.insert("Anthropic-Beta".to_owned(), "upper-beta".to_owned());
    let call = messages_call(model, USER_MESSAGES, params, request_headers);
    let body = session
        .transform(&call, &mut headers)
        .expect("the beta request transforms");
    assert_eq!(
        headers.get("Anthropic-Beta").map(String::as_str),
        Some("upper-beta")
    );
    assert!(!body.contains_key("anthropic_beta"));
    (headers, body)
}

fn beta_shapes() {
    beta_lists_join();
    beta_shape_errors();
}

fn beta_lists_join() {
    let mut headers = IndexMap::new();
    headers.insert("anthropic-beta".to_owned(), "caller-beta".to_owned());
    headers.insert("Anthropic-Beta".to_owned(), "upper-beta".to_owned());
    let mut body = obj(r#"{"anthropic_beta":["b1","b1","b2"]}"#);
    move_betas_into_header(&body, &mut headers).expect("the betas move");
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some("b1,b1,b2")
    );
    assert_eq!(
        headers.get("Anthropic-Beta").map(String::as_str),
        Some("upper-beta")
    );
    for falsey in [r#"[]"#, "null", "false", "0", "0.0", "-0.0", r#""""#, "{}"] {
        let mut body = obj(r#"{"anthropic_beta":[]}"#);
        body.insert("anthropic_beta".to_owned(), js(falsey));
        headers.insert("anthropic-beta".to_owned(), "caller-beta".to_owned());
        move_betas_into_header(&body, &mut headers).expect("a falsey list means no betas");
        assert!(!headers.contains_key("anthropic-beta"));
    }
}

fn beta_shape_errors() {
    let mut headers = IndexMap::new();
    let mut body = obj(r#"{"anthropic_beta":"b1"}"#);
    let error =
        move_betas_into_header(&body, &mut headers).expect_err("a string is not a tuple of betas");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "anthropic_beta must be a list of strings"
    );
    let mut body = obj(r#"{"anthropic_beta":["b1",2]}"#);
    let error =
        move_betas_into_header(&body, &mut headers).expect_err("a number is not a beta name");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(error.to_string(), "anthropic_beta[1] is not a string");
    let mut body = obj(r#"{"anthropic_beta":5}"#);
    let error =
        move_betas_into_header(&body, &mut headers).expect_err("a number is not a beta list");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "anthropic_beta must be a list of strings"
    );
    let mut body = obj(r#"{"anthropic_beta":{"a":"b"}}"#);
    let error =
        move_betas_into_header(&body, &mut headers).expect_err("an object is not a beta list");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "anthropic_beta must be a list of strings"
    );
}

fn restored_message_copy_semantics() {
    let mut body = obj(
        r#"{"stream":false,"max_tokens":8,"anthropic_version":"caller","anthropic_beta":["b1"]}"#,
    );
    restore_messages_fields(&mut body, "", None);
    assert!(!body.contains_key("model"));
    assert_eq!(body.get("stream"), Some(&BodyJson::Bool(false)));
    assert!(!body.contains_key("anthropic_version"));
    assert!(!body.contains_key("anthropic_beta"));
    assert_eq!(
        body.get("max_tokens"),
        Some(&BodyJson::Number(Number::from(8_u8)))
    );
    let stream = BodyJson::Bool(true);
    restore_messages_fields(&mut body, "claude", Some(&stream));
    assert_eq!(
        body.get("model"),
        Some(&BodyJson::Text("claude".to_owned()))
    );
    assert_eq!(body.get("stream"), Some(&BodyJson::Bool(true)));
}

pub(super) fn model_body_restoration() {
    restored_model_anywhere();
    restored_stream_flag();
    restored_copy_semantics();
}

fn restored_model_anywhere() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    let fetcher = MapFetcher::default();
    let rows = [
        (
            "fixture-mantle/plain",
            r#"{"max_tokens":64}"#,
            "bedrock-2023-05-31",
        ),
        (
            "fixture-plain",
            r#"{"max_tokens":64}"#,
            "bedrock-2023-05-31",
        ),
        (
            "fixture-plain",
            r#"{"max_tokens":64,"anthropic_version":"caller-version"}"#,
            "caller-version",
        ),
    ];
    for (model, params, version) in rows {
        let request = fixture.chat_request(model, prompt_messages(), map_of(params));
        let body = session
            .transform(&request, &fetcher, Mode::Sync)
            .expect("the model restores");
        assert_eq!(body.get("model"), Some(&js(r#""fixture-plain""#)));
        assert_eq!(body.get("max_tokens"), Some(&js("64")));
        assert_eq!(
            body.get("anthropic_version"),
            Some(&BodyJson::Text(version.to_owned()))
        );
        assert!(!body.contains_key("stream"));
    }
}

fn restored_stream_flag() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    let fetcher = MapFetcher::default();
    let request = fixture.chat_request(
        "mantle/fixture-plain",
        prompt_messages(),
        map_of(r#"{"max_tokens":64,"stream":true}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the stream request transforms");
    assert_eq!(body.get("stream"), Some(&BodyJson::Bool(true)));
    let request = fixture.chat_request(
        "mantle/fixture-plain",
        prompt_messages(),
        map_of(r#"{"max_tokens":64,"stream":false}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the unstreamed request transforms");
    assert!(!body.contains_key("stream"));
    let request = fixture.chat_request(
        "mantle/fixture-plain",
        prompt_messages(),
        map_of(r#"{"max_tokens":64,"stream":"true"}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("a string stream flag transforms");
    assert!(!body.contains_key("stream"));
}

fn restored_copy_semantics() {
    let mut body = obj(r#"{"stream":false,"max_tokens":64}"#);
    restore_body_fields(&mut body, "", None);
    assert_eq!(body.get("model"), Some(&BodyJson::Text(String::new())));
    assert_eq!(body.get("stream"), Some(&BodyJson::Bool(false)));
    assert_eq!(
        body.get("max_tokens"),
        Some(&BodyJson::Number(Number::from(64_u8)))
    );
    let stream = ParamJson::Bool(true);
    restore_body_fields(&mut body, "claude", Some(&stream));
    assert_eq!(body.get("stream"), Some(&BodyJson::Bool(true)));
    assert_eq!(
        body.get("model"),
        Some(&BodyJson::Text("claude".to_owned()))
    );
}

pub(super) fn mantle_capability_params() {
    effort_ceiling_clamped();
    structured_output_kept_or_inlined();
}

fn effort_ceiling_clamped() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    let fetcher = MapFetcher::default();
    let request = fixture.chat_request(
        "mantle/fixture-effort-tier",
        prompt_messages(),
        map_of(r#"{"max_tokens":64,"output_config":{"effort":"xhigh"}}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the effort request transforms");
    assert_eq!(at(&body, &["output_config", "effort"]), &js(r#""high""#));
    let request = fixture.chat_request(
        "mantle/fixture-plain",
        prompt_messages(),
        map_of(r#"{"max_tokens":64,"output_config":{"effort":"high"}}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the unsupported effort request transforms");
    assert!(!body.contains_key("output_config"));
}

fn structured_output_kept_or_inlined() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    let fetcher = MapFetcher::default();
    let request = fixture.chat_request(
        "mantle/fixture-structured",
        prompt_messages(),
        map_of(STRUCTURED_PARAMS),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the supported structured request transforms");
    assert_eq!(
        at(&body, &["output_config", "format", "type"]),
        &js(r#""json_schema""#)
    );
    let request = fixture.chat_request(
        "mantle/fixture-plain",
        prompt_messages(),
        map_of(STRUCTURED_PARAMS),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Sync)
        .expect("the unsupported structured request transforms");
    assert!(!body.contains_key("output_config"));
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef)
}

fn fixed_now() -> i64 {
    1_700_000_000
}

fn anthropic(chunk: InvokeChunk) -> StreamEventChunk {
    match chunk {
        InvokeChunk::Anthropic(parsed) => *parsed,
        InvokeChunk::Base(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            panic!("expected the Anthropic projection")
        }
    }
}

fn stream_event(text: &str) -> ParamJson {
    serde_json::from_str(text).expect("the stream event parses")
}

pub(super) fn mantle_native_sse() {
    chat_iterator_cases();
    messages_iterator_case();
}

fn chat_iterator_cases() {
    assert!(!HAS_CUSTOM_STREAM_WRAPPER);
    let clock = DecoderClock {
        next_id: fixed_id,
        now: fixed_now,
    };
    let mut decoder = model_response_iterator("claude", false, None, clock);
    let start = anthropic(
        decoder
            .chunk_parser(&stream_event(
                r#"{"type":"message_start","message":{"model":"claude","usage":{"input_tokens":1,"output_tokens":2}}}"#,
            ))
            .expect("the native start parses"),
    );
    assert!(start.usage.is_some());
    assert_eq!(start.model.as_deref(), Some("claude"));
    let text = anthropic(
        decoder
            .chunk_parser(&stream_event(
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            ))
            .expect("the native delta parses"),
    );
    assert_eq!(text.choices[0].delta.content, "hi");
    assert_eq!(text.id, format!("chatcmpl-{}", fixed_id()));
    let mut suppressed = model_response_iterator("claude", true, Some(true), clock);
    let tool = anthropic(
        suppressed
            .chunk_parser(&stream_event(
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"j","name":"json_tool_call","input":{}}}"#,
            ))
            .expect("the json-mode start parses"),
    );
    assert!(tool.choices[0].delta.tool_calls.is_none());
    assert_eq!(tool.choices[0].delta.content, "");
}

fn messages_iterator_case() {
    let runtime = runtime();
    let logging_obj = logging_object();
    let seams = Arc::new(Recorder {
        processed: Mutex::new(None),
        supplied: Arc::clone(&logging_obj),
    });
    let body = obj(r#"{"model":"claude","max_tokens":8,"stream":true}"#);
    runtime.block_on(async {
        let relayed = messages_iterator(
            recorder_deps(Arc::clone(&seams)),
            "native sse".to_owned(),
            &body,
            Arc::clone(&logging_obj),
        )
        .expect("the native iterator builds");
        assert_eq!(relayed, "processed native sse");
    });
    let processed = seams
        .processed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("the chunk processor ran");
    assert_eq!(processed.response, "native sse");
    assert_eq!(processed.request_body, contract_of(&body));
    assert!(processed.same_logging_object);
    assert_eq!(processed.url_route, "/v1/messages");
    assert!(processed.endpoint_anthropic);
}

fn response_fields(text: &str) -> Fields {
    transform_response(text.as_bytes(), 200, &LIMITS).expect("the response transforms")
}

fn usage_of(fields: &Fields) -> BodyJson {
    serde_json::from_str(fields.get("usage").expect("the usage exists").get())
        .expect("the usage parses")
}

pub(super) fn mantle_usage_fields() {
    usage_defaults_and_preservation();
    usage_failures();
}

fn usage_defaults_and_preservation() {
    let fields = response_fields(
        r#"{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude","stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":4}}"#,
    );
    assert_eq!(
        usage_of(&fields),
        js(r#"{"input_tokens":3,"output_tokens":4}"#)
    );
    let fields = response_fields(r#"{"id":"msg_2","type":"message","model":"other","content":[]}"#);
    assert_eq!(
        usage_of(&fields),
        js(r#"{"input_tokens":0,"output_tokens":0}"#)
    );
    assert_eq!(fields.get("model").map(|raw| raw.get()), Some(r#""other""#));
    let preserved = r#"{"input_tokens":null,"output_tokens":null,"cache_read_input_tokens":7,"cache_creation_input_tokens":2,"server_tool_use":{"web_search_requests":1},"extra":"x"}"#;
    let fields = response_fields(&format!(
        r#"{{"id":"msg_3","type":"message","usage":{preserved}}}"#
    ));
    assert_eq!(usage_of(&fields), js(preserved));
    for falsey in ["null", "false", "0", r#""""#, "[]"] {
        let fields = response_fields(&format!(
            r#"{{"id":"msg_4","type":"message","usage":{falsey}}}"#
        ));
        assert_eq!(
            usage_of(&fields),
            js(r#"{"input_tokens":0,"output_tokens":0}"#)
        );
    }
}

fn usage_failures() {
    let error = transform_response(br#"{"usage":"many"}"#, 200, &LIMITS)
        .expect_err("a truthy non-mapping usage fails");
    assert_eq!(error.status_code(), Some(500));
    assert_eq!(
        error.to_string(),
        "the Messages response usage is not an object"
    );
    let error =
        transform_response(br#"{"usage":[1]}"#, 200, &LIMITS).expect_err("a list usage fails");
    assert_eq!(error.status_code(), Some(500));
    let error = transform_response(br#"{"usage":5}"#, 200, &LIMITS)
        .expect_err("a truthy number usage fails");
    assert_eq!(error.status_code(), Some(500));
    let error = transform_response(b"bad gateway", 502, &LIMITS)
        .expect_err("a non-JSON body keeps the raw status");
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(error.to_string(), "bad gateway");
    let error = transform_response(b"[1,2]", 200, &LIMITS).expect_err("a non-object body fails");
    assert_eq!(error.status_code(), Some(500));
}

pub(super) fn at<'value>(body: &'value Object, path: &[&str]) -> &'value BodyJson {
    let (first, rest) = path.split_first().expect("the path is not empty");
    let mut current = body.get(*first).expect("the body field exists");
    for step in rest {
        current = match current {
            BodyJson::Object(fields) => fields.get(*step).expect("the object field exists"),
            BodyJson::List(items) => items
                .get(step.parse().expect("a list step is an index"))
                .expect("the list index exists"),
            BodyJson::Null | BodyJson::Bool(_) | BodyJson::Number(_) | BodyJson::Text(_) => {
                panic!("the path meets a scalar")
            }
        };
    }
    current
}

const MEDIA_MESSAGES: &str = r#"[{"role":"user","content":[
    {"type":"image_url","image_url":{"url":"http://example.test/pic.png"}},
    {"type":"document","source":{"type":"url","url":"http://example.test/doc.pdf"}}
]}]"#;

pub(super) const SYNC_MESSAGES: &str = r#"[{"role":"user","content":[
    {"type":"file","file":{"file_id":"https://files.example.test/f1"}},
    {"type":"image_url","image_url":{"url":"https://example.test/pic.png"}}
]}]"#;

const MEDIA_URLS: &[(&str, &str)] = &[
    (
        "http://example.test/pic.png",
        "data:image/png;base64,iVBORw0KGgo=",
    ),
    (
        "http://example.test/doc.pdf",
        "data:application/pdf;base64,JVBERi0xLjQK",
    ),
];

pub(super) fn remote_url_inlined() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    let fetcher = MapFetcher::new(MEDIA_URLS);
    let body = media_body(
        &fixture,
        &mut session,
        &fetcher,
        MEDIA_MESSAGES,
        Mode::Async,
    );
    assert_sources(
        &body,
        [
            IMAGE_BODY,
            r#"{"data":"JVBERi0xLjQK","media_type":"application/pdf","type":"base64"}"#,
        ],
    );
    assert_eq!(
        fetcher.calls(),
        vec!["http://example.test/pic.png", "http://example.test/doc.pdf"]
    );
    let quiet = MapFetcher::default();
    let body = media_body(&fixture, &mut session, &quiet, SYNC_MESSAGES, Mode::Sync);
    assert!(quiet.calls().is_empty());
    assert_sources(
        &body,
        [
            r#"{"type":"url","url":"https://files.example.test/f1"}"#,
            r#"{"type":"url","url":"https://example.test/pic.png"}"#,
        ],
    );
}

fn media_body(
    fixture: &Fixture,
    session: &mut ChatSession<'_>,
    fetcher: &MapFetcher,
    messages: &str,
    mode: Mode,
) -> Object {
    let request = fixture.chat_request(
        "fixture-plain",
        messages_of(messages),
        map_of(r#"{"max_tokens":64}"#),
    );
    session
        .transform(&request, fetcher, mode)
        .expect("the media request transforms")
}

// Both rounds keep the message order, so the sources sit at content 0 and 1.
fn assert_sources(body: &Object, expected: [&str; 2]) {
    for (index, source) in expected.iter().enumerate() {
        let position = index.to_string();
        let path = ["messages", "0", "content", position.as_str(), "source"];
        assert_eq!(at(body, &path), &js(source));
    }
}

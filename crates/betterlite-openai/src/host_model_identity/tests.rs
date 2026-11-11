#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;
use uuid::Uuid;

use super::error::HostModelIdentityError;
use super::{
    HostReply, HostReplyMode, MAX_REPLY_BYTES, ReplyDefaults, SUPPORTED_OPENAI_PARAMS,
    clarifai_base_model, clarifai_request_model, datarobot_api_base, decode_host_named_reply,
    process_host_response,
};
use crate::chat::response::{ChatResponse, ChatUsage};
use crate::compatible_wire::LogInput;
use crate::openai_base_config::Headers;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn fixed_clock() -> i64 {
    1_700_000_000
}

fn negative_clock() -> i64 {
    -1
}

const DEFAULTS: ReplyDefaults = ReplyDefaults {
    identifier: fixed_id,
    clock: fixed_clock,
};

struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0.lock().unwrap().push(record);
    }
}

fn logged<T, A>(action: A) -> (T, LoggingState)
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .unwrap();
    let sink = Sink(Mutex::new(Vec::new()));
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let result = action(&mut state, &env);
    (result, state)
}

fn log_input(messages: &str, data: &[(&str, &str)]) -> LogInput {
    LogInput {
        messages: serde_json::from_str(messages).unwrap(),
        data: RawBody::Object(
            data.iter()
                .map(|(name, text)| ((*name).to_owned(), Json::Text((*text).to_owned())))
                .collect::<IndexMap<String, Json>>(),
        ),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn decode(bytes: &[u8], mode: HostReplyMode<'_>) -> Result<ChatResponse, HostModelIdentityError> {
    decode_host_named_reply(bytes, mode, DEFAULTS)
}

fn clarifai_mode() -> HostReplyMode<'static> {
    HostReplyMode::Clarifai {
        request_model: "user.app.model",
    }
}

fn compactifai_mode(json_mode: bool) -> HostReplyMode<'static> {
    HostReplyMode::Compactifai {
        request_model: "user.app.model",
        json_mode,
    }
}

fn reply_body(model: &str, message: &str, usage: &str) -> String {
    format!(
        r#"{{"id":"reply","created":7,"model":{model},"choices":[{{"index":0,"message":{message},"finish_reason":"stop"}}],"usage":{usage}}}"#
    )
}

const PLAIN: &str = r#"{"role":"assistant","content":"hi"}"#;
const USAGE: &str = r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#;

#[test]
fn clarifai_advertises_its_openai_params_in_litellm_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "max_completion_tokens",
            "response_format",
            "stream",
            "temperature",
            "top_p",
            "tool_choice",
            "tools",
            "presence_penalty",
            "frequency_penalty",
            "stream_options",
        ]
    );
}

#[rstest]
#[case::dotted("user.app.model", "https://clarifai.com/user/app/models/model")]
#[case::empty_app("user..model", "https://clarifai.com/user//models/model")]
#[case::empty_user(".app.model", "https://clarifai.com//app/models/model")]
#[case::empty_model("user.app.", "https://clarifai.com/user/app/models/")]
fn clarifai_base_model_accepts_three_dotted_components(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        clarifai_base_model(Some(model)).unwrap(),
        Some(expected.to_owned())
    );
}

#[rstest]
#[case::two_components("user.app")]
#[case::four_components("a.b.c.d")]
#[case::trailing_dot("user.app.model.")]
fn clarifai_base_model_rejects_other_arities(#[case] model: &str) {
    let error = clarifai_base_model(Some(model)).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::ModelName { .. }));
}

#[test]
fn clarifai_base_model_keeps_an_absent_or_empty_model() {
    assert_eq!(clarifai_base_model(None).unwrap(), None);
    assert_eq!(clarifai_base_model(Some("")).unwrap(), None);
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::converted(
    Some("user.app.model"),
    Some("https://clarifai.com/user/app/models/model")
)]
fn clarifai_request_model_keeps_a_falsey_model_unchanged(
    #[case] model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(clarifai_request_model(model).unwrap().as_deref(), expected);
}

#[rstest]
#[case::given("\"provider-model\"", "clarifai/user.app.model")]
#[case::null("null", "")]
fn clarifai_replaces_only_a_non_null_response_model(#[case] model: &str, #[case] expected: &str) {
    let response = decode(reply_body(model, PLAIN, USAGE).as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.model, expected);
}

#[test]
fn clarifai_reads_an_absent_response_model_as_empty() {
    let body = format!(r#"{{"choices":[{{"message":{PLAIN}}}]}}"#);
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.model, "");
}

#[rstest]
#[case::given("\"provider-model\"", "compactifai/user.app.model")]
#[case::null("null", "compactifai/user.app.model")]
fn compactifai_always_prefixes_the_response_model(#[case] model: &str, #[case] expected: &str) {
    let response = decode(
        reply_body(model, PLAIN, USAGE).as_bytes(),
        compactifai_mode(false),
    )
    .unwrap();
    assert_eq!(response.model, expected);
}

const ONE_TOOL: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}]}"#;
const NULL_ARGUMENTS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":null}}]}"#;
const MISSING_ARGUMENTS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f"}}]}"#;
const TWO_TOOLS: &str = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"t","type":"function","function":{"name":"f","arguments":"x"}},{"id":"u","type":"function","function":{"name":"g","arguments":"y"}}]}"#;
const NO_TOOLS: &str = r#"{"role":"assistant","content":"kept","tool_calls":[]}"#;

#[rstest]
#[case::arguments_become_content(ONE_TOOL, Some("{\"a\":1}"), None)]
#[case::null_arguments_stay_null(NULL_ARGUMENTS, None, None)]
#[case::missing_arguments_become_empty(MISSING_ARGUMENTS, Some(""), None)]
#[case::two_calls_stay(TWO_TOOLS, None, Some(2))]
#[case::no_calls_stay(NO_TOOLS, Some("kept"), None)]
fn compactifai_json_mode_moves_exactly_one_tool_call(
    #[case] message: &str,
    #[case] content: Option<&str>,
    #[case] tool_count: Option<usize>,
) {
    let response = decode(
        reply_body("\"upstream\"", message, USAGE).as_bytes(),
        compactifai_mode(true),
    )
    .unwrap();
    let delivered = &response.choices[0].message;
    assert_eq!(delivered.content.as_deref(), content);
    assert_eq!(delivered.tool_calls.as_ref().map(Vec::len), tool_count);
}

#[test]
fn compactifai_keeps_one_tool_call_without_json_mode() {
    let response = decode(
        reply_body("\"upstream\"", ONE_TOOL, USAGE).as_bytes(),
        compactifai_mode(false),
    )
    .unwrap();
    let delivered = &response.choices[0].message;
    assert_eq!(delivered.content, None);
    assert_eq!(delivered.tool_calls.as_ref().map(Vec::len), Some(1));
}

#[test]
fn a_null_usage_reads_as_the_zero_default() {
    let response = decode(
        reply_body("\"m\"", PLAIN, "null").as_bytes(),
        clarifai_mode(),
    )
    .unwrap();
    assert_eq!(response.usage, ChatUsage::default());
}

#[test]
fn an_absent_usage_reads_as_the_zero_default() {
    let body = format!(r#"{{"model":"m","choices":[{{"message":{PLAIN}}}]}}"#);
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.usage, ChatUsage::default());
}

#[test]
fn null_token_counts_read_as_zero() {
    let usage = r#"{"prompt_tokens":null,"completion_tokens":5,"total_tokens":null}"#;
    let response = decode(
        reply_body("\"m\"", PLAIN, usage).as_bytes(),
        clarifai_mode(),
    )
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 0);
}

#[rstest]
#[case::null("null")]
#[case::absent("")]
fn a_missing_or_null_choices_list_becomes_one_default_choice(#[case] choices_value: &str) {
    let choices_field = if choices_value.is_empty() {
        String::new()
    } else {
        format!(r#","choices":{choices_value}"#)
    };
    let body = format!(r#"{{"model":"m"{choices_field}}}"#);
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    let delivered = &response.choices;
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].index, 0);
    assert_eq!(delivered[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(delivered[0].message.role, "assistant");
    assert_eq!(delivered[0].message.content, None);
}

#[rstest]
#[case::absent_index_and_role(r#"{"choices":[{"message":{"content":"hi"}}]}"#, 0)]
#[case::null_index_and_role(
    r#"{"choices":[{"index":null,"message":{"role":null,"content":"hi"}}]}"#,
    0
)]
#[case::explicit_index(r#"{"choices":[{"index":3,"message":{"role":"","content":"hi"}}]}"#, 3)]
fn an_absent_or_null_choice_index_and_role_take_the_default(
    #[case] body: &str,
    #[case] index: u32,
) {
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.choices[0].index, index);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
}

#[test]
fn a_mapped_finish_reason_keeps_the_native_value() {
    let body = r#"{"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"max_tokens"}]}"#;
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    let choice = &response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        choice.provider_specific_fields.get("native_finish_reason"),
        Some(&Value::String(String::from("max_tokens")))
    );
}

#[test]
fn choices_that_are_not_a_list_are_refused() {
    let body = r#"{"model":"m","choices":{"index":0}}"#;
    let error = decode(body.as_bytes(), clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Reply { .. }));
}

#[test]
fn an_exact_finish_reason_stays_as_sent() {
    let body = r#"{"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
}

#[rstest]
#[case::message_content_object(
    r#"{"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":{"a":1}}}]}"#
)]
#[case::index_text(
    r#"{"model":"m","choices":[{"index":"zero","message":{"role":"assistant","content":"hi"}}]}"#
)]
fn reply_field_types_are_checked(#[case] body: &str) {
    let error = decode(body.as_bytes(), clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Reply { .. }));
}

#[test]
fn a_choices_entry_that_is_not_an_object_is_refused() {
    let body = r#"{"model":"m","choices":[[]]}"#;
    let error = decode(body.as_bytes(), clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Reply { .. }));
}

#[rstest]
#[case::array(b"[]")]
#[case::text(b"\"text\"")]
#[case::number(b"3")]
fn a_reply_body_that_is_not_an_object_is_malformed(#[case] body: &[u8]) {
    let error = decode(body, clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Malformed { .. }));
}

#[rstest]
#[case::clarifai(clarifai_mode(), "the clarifai reply is not a chat completion object")]
#[case::compactifai(
    compactifai_mode(false),
    "the compactifai reply is not a chat completion object"
)]
fn a_malformed_reply_names_its_host(#[case] mode: HostReplyMode<'static>, #[case] message: &str) {
    let error = decode(b"[]", mode).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn a_reply_at_the_transport_limit_is_not_too_large() {
    let body = vec![b' '; MAX_REPLY_BYTES];
    let error = decode(&body, clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Json { .. }));
}

#[test]
fn a_reply_over_the_transport_limit_is_refused() {
    let body = vec![b' '; MAX_REPLY_BYTES + 1];
    let error = decode(&body, clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::TooLarge));
}

fn failing_clarifai_reply() -> (LoggingState, Headers, HostModelIdentityError) {
    let supplied = headers(&[("x-request-id", "req-1"), ("x-trace", "trace-1")]);
    let reply = HostReply {
        mode: clarifai_mode(),
        defaults: DEFAULTS,
        status_code: 400,
        headers: &supplied,
    };
    let (outcome, state) = logged(|state, env| {
        process_host_response(
            "not json at all",
            &reply,
            log_input(r#"[{"role":"user","content":"hello"}]"#, &[("model", "m")]),
            state,
            env,
        )
    });
    (state, supplied, outcome.unwrap_err())
}

#[test]
fn clarifai_logs_the_raw_reply_before_it_parses() {
    let (state, _supplied, _error) = failing_clarifai_reply();
    let details = &state.model_call_details;
    assert_eq!(
        details["original_response"],
        Json::Text("not json at all".to_owned())
    );
    assert_eq!(
        serde_json::to_value(&details["input"]).unwrap(),
        value(r#"[{"role":"user","content":"hello"}]"#)
    );
    assert_eq!(
        serde_json::to_value(&details["additional_args"]).unwrap(),
        value(r#"{"complete_input_dict":{"model":"m"}}"#)
    );
    assert_eq!(
        details["log_event_type"],
        Json::Text("post_api_call".to_owned())
    );
}

#[test]
fn a_clarifai_json_failure_carries_the_status_and_headers() {
    let (_state, supplied, error) = failing_clarifai_reply();
    assert_eq!(
        error.to_string(),
        "the Clarifai response body is not valid JSON"
    );
    assert!(error.source().is_some());
    let HostModelIdentityError::ClarifaiJson {
        status_code,
        headers,
        ..
    } = error
    else {
        panic!("expected the wrapped OpenAI error");
    };
    assert_eq!(status_code, 400);
    assert_eq!(headers, supplied);
}

#[test]
fn compactifai_logs_the_raw_reply_before_its_json_failure() {
    let body = r#"{"choices": [}"#;
    let reply = HostReply {
        mode: compactifai_mode(false),
        defaults: DEFAULTS,
        status_code: 200,
        headers: &Headers::new(),
    };
    let (outcome, state) =
        logged(|state, env| process_host_response(body, &reply, log_input("[]", &[]), state, env));
    assert_eq!(
        state.model_call_details["original_response"],
        Json::Text(body.to_owned())
    );
    assert!(matches!(
        outcome.unwrap_err(),
        HostModelIdentityError::Json { .. }
    ));
}

#[test]
fn a_valid_reply_logs_the_same_call_before_it_is_returned() {
    let body = reply_body("\"upstream\"", PLAIN, USAGE);
    let reply = HostReply {
        mode: clarifai_mode(),
        defaults: DEFAULTS,
        status_code: 200,
        headers: &Headers::new(),
    };
    let (outcome, state) =
        logged(|state, env| process_host_response(&body, &reply, log_input("[]", &[]), state, env));
    let response = outcome.unwrap();
    assert_eq!(
        state.model_call_details["original_response"],
        Json::Text(body)
    );
    assert_eq!(response.model, "clarifai/user.app.model");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
}

#[test]
fn a_reply_without_id_or_created_takes_the_injected_defaults() {
    let body = format!(r#"{{"choices":[{{"index":0,"message":{PLAIN}}}]}}"#);
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert_eq!(response.id, "chatcmpl-00000000-0000-0000-0000-000000000000");
    assert_eq!(response.created, 1_700_000_000);
    let given = decode(
        reply_body("\"m\"", PLAIN, USAGE).as_bytes(),
        clarifai_mode(),
    )
    .unwrap();
    assert_eq!(given.id, "reply");
    assert_eq!(given.created, 7);
}

#[test]
fn a_clock_reading_below_zero_is_an_error() {
    let body = format!(r#"{{"choices":[{{"index":0,"message":{PLAIN}}}]}}"#);
    let defaults = ReplyDefaults {
        identifier: fixed_id,
        clock: negative_clock,
    };
    let error = decode_host_named_reply(body.as_bytes(), clarifai_mode(), defaults).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::Clock { .. }));
}

#[rstest]
#[case::empty("", "/api/v2/genai/llmgw/chat/completions/")]
#[case::root("/", "/api/v2/genai/llmgw/chat/completions/")]
#[case::default_host(
    "https://app.datarobot.com",
    "https://app.datarobot.com/api/v2/genai/llmgw/chat/completions/"
)]
#[case::standard_endpoint(
    "https://tenant.example/api/v2",
    "https://tenant.example/api/v2/genai/llmgw/chat/completions/"
)]
#[case::deployment_is_kept(
    "https://tenant.example/api/v2/deployments/68f1",
    "https://tenant.example/api/v2/deployments/68f1/"
)]
#[case::llmgw_is_kept(
    "https://tenant.example/api/v2/genai/llmgw/chat/completions",
    "https://tenant.example/api/v2/genai/llmgw/chat/completions/"
)]
#[case::query_and_fragment_are_kept(
    "https://tenant.example/api/v2?limit=5#frag",
    "https://tenant.example/api/v2/genai/llmgw/chat/completions/?limit=5#frag"
)]
#[case::double_slashes_collapse(
    "https://tenant.example//api//v2//",
    "https://tenant.example/api/v2/"
)]
#[case::triple_slashes_keep_a_pair(
    "https://tenant.example///api///v2///",
    "https://tenant.example//api//v2//"
)]
fn datarobot_edits_only_the_path(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(datarobot_api_base(api_base).unwrap(), expected);
}

fn corpus_mode(tag: u8) -> HostReplyMode<'static> {
    if tag.is_multiple_of(2) {
        HostReplyMode::Clarifai {
            request_model: "user.app.model",
        }
    } else {
        HostReplyMode::Compactifai {
            request_model: "user.app.model",
            json_mode: tag & 2 != 0,
        }
    }
}

#[rstest]
fn replays_host_named_reply_corpus(#[files("fuzz/corpus/host_named_reply/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let (tag, body) = bytes.split_first().unwrap();
    let mode = corpus_mode(*tag);
    let name = path.file_name().unwrap().to_str().unwrap();
    let prefix = match mode {
        HostReplyMode::Clarifai { .. } => "clarifai_",
        HostReplyMode::Compactifai {
            json_mode: true, ..
        } => "compactifai_json_",
        HostReplyMode::Compactifai {
            json_mode: false, ..
        } => "compactifai_plain_",
    };
    assert!(
        name.starts_with(prefix),
        "{name} does not match its mode tag"
    );
    let _decoded = decode_host_named_reply(body, mode, DEFAULTS);
}

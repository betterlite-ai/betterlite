#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use uuid::Uuid;

use super::error::{HostModelIdentityError, get_error_class};
use super::identity::{
    SUPPORTED_OPENAI_PARAMS, clarifai_base_model, clarifai_request_model, datarobot_api_base,
};
use super::{
    HostReply, HostReplyMode, MAX_REPLY_BYTES, PostCall, PostCallLogger, ReplySource,
    decode_host_named_reply, process_clarifai_response, process_compactifai_response,
};
use crate::chat::response::{ChatResponse, ChatUsage};
use crate::openai_base_config::{Headers, Object};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(Object::from_iter(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value)),
    ))
}

#[derive(Default)]
struct Recorder(Vec<Value>);

impl PostCallLogger for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        let api_key = call.api_key.map_or(Value::Null, |key| {
            Value::String(key.expose_secret().to_owned())
        });
        self.0.push(object(vec![
            ("input", serde_json::to_value(call.input).unwrap()),
            ("api_key", api_key),
            (
                "original_response",
                Value::String(call.original_response.to_owned()),
            ),
            (
                "complete_input_dict",
                Value::Object(call.complete_input_dict.clone()),
            ),
        ]));
    }
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

fn fixed_clock() -> i64 {
    1_700_000_000
}

fn source() -> ReplySource {
    ReplySource {
        identifier: fixed_id,
        clock: fixed_clock,
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn messages(value: Value) -> Json {
    serde_json::from_value(value).unwrap()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn decode(bytes: &[u8], mode: HostReplyMode<'_>) -> Result<ChatResponse, HostModelIdentityError> {
    decode_host_named_reply(bytes, mode, &source())
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
#[case::an_object("{\"index\":0}")]
fn choices_that_are_not_a_list_become_one_default_choice(#[case] choices_value: &str) {
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
fn an_exact_finish_reason_keeps_no_native_value() {
    let body = r#"{"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
    let response = decode(body.as_bytes(), clarifai_mode()).unwrap();
    assert!(response.choices[0].provider_specific_fields.is_empty());
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

#[test]
fn a_reply_over_the_transport_limit_is_refused() {
    let body = vec![b' '; MAX_REPLY_BYTES + 1];
    let error = decode(&body, clarifai_mode()).unwrap_err();
    assert!(matches!(error, HostModelIdentityError::TooLarge));
}

fn failing_clarifai_reply() -> (Recorder, Headers, HostModelIdentityError) {
    let body = "not json at all";
    let supplied = headers(&[("x-request-id", "req-1"), ("x-trace", "trace-1")]);
    let key = secret("request-key");
    let input = messages(value(r#"[{"role":"user","content":"hello"}]"#));
    let request_data =
        Object::from_iter([(String::from("model"), Value::String(String::from("m")))]);
    let mut recorder = Recorder::default();
    let error = process_clarifai_response(
        HostReply {
            body,
            status_code: 400,
            headers: &supplied,
            request_model: "user.app.model",
            messages: &input,
            api_key: Some(&key),
            request_data: &request_data,
            json_mode: false,
            source: source(),
        },
        &mut recorder,
    )
    .unwrap_err();
    (recorder, supplied, error)
}

#[test]
fn clarifai_logs_the_raw_reply_before_it_parses() {
    let (recorder, _supplied, _error) = failing_clarifai_reply();
    assert_eq!(recorder.0.len(), 1);
    assert_eq!(recorder.0[0]["original_response"], "not json at all");
    assert_eq!(recorder.0[0]["api_key"], "request-key");
    assert_eq!(
        recorder.0[0]["input"],
        value(r#"[{"role":"user","content":"hello"}]"#)
    );
    assert_eq!(
        recorder.0[0]["complete_input_dict"],
        value(r#"{"model":"m"}"#)
    );
}

#[test]
fn a_clarifai_json_failure_carries_the_status_and_headers() {
    let (_recorder, supplied, error) = failing_clarifai_reply();
    let HostModelIdentityError::Api {
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
    let key = secret("request-key");
    let input = messages(value("[]"));
    let mut recorder = Recorder::default();
    let error = process_compactifai_response(
        HostReply {
            body,
            status_code: 200,
            headers: &Headers::new(),
            request_model: "user.app.model",
            messages: &input,
            api_key: Some(&key),
            request_data: &Object::new(),
            json_mode: false,
            source: source(),
        },
        &mut recorder,
    )
    .unwrap_err();
    assert_eq!(recorder.0.len(), 1);
    assert_eq!(recorder.0[0]["original_response"], body);
    assert!(matches!(error, HostModelIdentityError::Json { .. }));
}

#[test]
fn a_valid_reply_logs_the_same_call_before_it_is_returned() {
    let body = reply_body("\"upstream\"", PLAIN, USAGE);
    let key = secret("request-key");
    let input = messages(value(r#"[{"role":"user","content":"hello"}]"#));
    let mut recorder = Recorder::default();
    let response = process_clarifai_response(
        HostReply {
            body: &body,
            status_code: 200,
            headers: &Headers::new(),
            request_model: "user.app.model",
            messages: &input,
            api_key: Some(&key),
            request_data: &Object::new(),
            json_mode: false,
            source: source(),
        },
        &mut recorder,
    )
    .unwrap();
    assert_eq!(recorder.0.len(), 1);
    assert_eq!(recorder.0[0]["original_response"], body);
    assert_eq!(response.model, "clarifai/user.app.model");
    assert_eq!(response.choices[0].message.content.as_deref(), Some("hi"));
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

#[test]
fn error_class_keeps_the_status_message_and_headers() {
    let supplied = headers(&[("x-request-id", "req-2")]);
    let error = get_error_class("rate limited", 429, &supplied);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rate limited");
    assert_eq!(error.headers, supplied);
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
    let _decoded = decode_host_named_reply(body, mode, &source());
}

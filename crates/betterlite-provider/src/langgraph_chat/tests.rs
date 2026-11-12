#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hint::black_box;
use std::process;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use tokio::runtime::{Builder, Runtime};
use tracing::Level;
use tracing::subscriber::with_default;

use super::config::{
    SHOULD_FAKE_STREAM, STREAM_PARAM_IN_BODY, SUPPORTED_OPENAI_PARAMS, get_complete_url,
    provider_info, validate_environment,
};
use super::dumps::dumps;
use super::error::{LangGraphError, LangGraphFailure, get_error_class};
use super::records::{
    LangGraphParsedResponse, LangGraphResponse, LangGraphRole, LangGraphStreamEvent, Usage,
};
use super::reply::{LangGraphReply, ReplyInput, TokenCounter, transform_response};
use super::request::{assistant_id, convert_messages, transform_request};

mod reply;

fn options(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn body_text(wire: &str, stream: bool, extra: &str) -> String {
    let request =
        transform_request("langgraph/agent", &messages(wire), &options(extra), stream).unwrap();
    serde_json::to_string(&request).unwrap()
}

#[rstest]
#[case::explicit_wins(Some("http://a"), Some("http://b"), "http://a")]
#[case::env_when_no_argument(None, Some("http://b"), "http://b")]
#[case::default_when_neither(None, None, "http://localhost:2024")]
#[case::empty_argument_is_unset(Some(""), Some("http://b"), "http://b")]
#[case::empty_env_is_unset(None, Some(""), "http://localhost:2024")]
fn provider_info_resolves_the_api_base(
    #[case] argument: Option<&str>,
    #[case] env: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(provider_info(argument, None, env, None).api_base, expected);
}

#[rstest]
#[case::explicit_wins(Some("a"), Some("b"), Some("a"))]
#[case::env_when_no_argument(None, Some("b"), Some("b"))]
#[case::none_when_neither(None, None, None)]
#[case::empty_argument_is_unset(Some(""), Some("b"), Some("b"))]
#[case::empty_env_is_unset(None, Some(""), None)]
fn provider_info_resolves_the_api_key(
    #[case] argument: Option<&str>,
    #[case] env: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let explicit = argument.map(secret);
    let from_env = env.map(secret);
    let info = provider_info(None, explicit.as_ref(), None, from_env.as_ref());
    assert_eq!(
        info.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected
    );
}

#[test]
fn only_the_stream_param_is_supported_and_the_body_has_no_stream_flag() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["stream"]);
    assert!(!black_box(STREAM_PARAM_IN_BODY));
    assert!(!black_box(SHOULD_FAKE_STREAM));
}

#[rstest]
#[case::wait("http://host:2024", false, "http://host:2024/runs/wait")]
#[case::stream("http://host:2024", true, "http://host:2024/runs/stream")]
#[case::one_slash("http://host/", false, "http://host/runs/wait")]
#[case::many_slashes("http://host///", true, "http://host/runs/stream")]
#[case::inner_slash_kept("http://host/api/", false, "http://host/api/runs/wait")]
fn complete_url_picks_the_endpoint(
    #[case] base: &str,
    #[case] stream: bool,
    #[case] expected: &str,
) {
    assert_eq!(get_complete_url(Some(base), stream).unwrap(), expected);
}

#[test]
fn complete_url_without_an_api_base_names_the_env_var() {
    let error = get_complete_url(None, false).unwrap_err();
    assert!(matches!(error, LangGraphFailure::MissingApiBase));
    assert_eq!(
        error.to_string(),
        "api_base is required for LangGraph. Set it via LANGGRAPH_API_BASE env var or api_base parameter."
    );
}

#[rstest]
#[case::with_key(Some("sk-1"), &[("Content-Type", "application/json"), ("Authorization", "Bearer sk-1")])]
#[case::without_key(None, &[("Content-Type", "application/json")])]
#[case::empty_key(Some(""), &[("Content-Type", "application/json")])]
fn headers_carry_json_and_the_bearer_key(
    #[case] key: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let api_key = key.map(secret);
    let mut headers = BTreeMap::from([(String::from("X-Other"), String::from("kept"))]);
    validate_environment(&mut headers, api_key.as_ref());
    let mut wanted: BTreeMap<String, String> = expected
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    wanted.insert(String::from("X-Other"), String::from("kept"));
    assert_eq!(headers, wanted);
}

#[test]
fn the_json_content_type_replaces_a_caller_value() {
    let mut headers = BTreeMap::from([(String::from("Content-Type"), String::from("text/plain"))]);
    validate_environment(&mut headers, None);
    assert_eq!(headers["Content-Type"], "application/json");
}

#[rstest]
#[case::option_first(r#"{"assistant_id":"given"}"#, "langgraph/agent", "given")]
#[case::model_suffix("{}", "langgraph/agent", "agent")]
#[case::whole_model("{}", "agent", "agent")]
#[case::split_on_the_first_slash("{}", "langgraph/team/agent", "team/agent")]
#[case::empty_option_falls_through(r#"{"assistant_id":""}"#, "langgraph/agent", "agent")]
#[case::null_option_falls_through(r#"{"assistant_id":null}"#, "langgraph/agent", "agent")]
fn assistant_id_sources(#[case] wire: &str, #[case] model: &str, #[case] expected: &str) {
    assert_eq!(assistant_id(model, &options(wire)).unwrap(), expected);
}

#[test]
fn a_non_text_assistant_id_is_rejected_by_name() {
    let error = assistant_id("agent", &options(r#"{"assistant_id":7}"#)).unwrap_err();
    assert!(matches!(
        error,
        LangGraphFailure::OptionType {
            field: "assistant_id"
        }
    ));
}

#[rstest]
#[case::user("user", LangGraphRole::Human)]
#[case::assistant("assistant", LangGraphRole::Assistant)]
#[case::system("system", LangGraphRole::System)]
#[case::tool("tool", LangGraphRole::Human)]
#[case::function("function", LangGraphRole::Human)]
#[case::developer("developer", LangGraphRole::Human)]
fn roles_map_to_human_assistant_and_system(#[case] role: &str, #[case] expected: LangGraphRole) {
    let wire = format!(r#"[{{"role":"{role}","content":"hi"}}]"#);
    let converted = convert_messages(&messages(&wire));
    assert_eq!(converted.len(), 1);
    assert_eq!(converted[0].role, expected);
    assert_eq!(converted[0].content, "hi");
}

#[test]
fn list_content_is_flattened_to_its_text() {
    let converted = convert_messages(&messages(
        r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"u"}},{"type":"text","text":"b"}]}]"#,
    ));
    assert_eq!(converted[0].content, "ab");
}

#[test]
fn absent_content_becomes_the_empty_string() {
    let converted = convert_messages(&messages(r#"[{"role":"assistant"}]"#));
    assert_eq!(converted[0].content, "");
}

#[rstest]
#[case::kept(r#"{"skillId":"s"}"#, true)]
#[case::empty_object_dropped("{}", false)]
#[case::null_dropped("null", false)]
#[case::non_object_dropped(r#""text""#, false)]
fn message_metadata_is_kept_only_when_it_is_a_non_empty_object(
    #[case] metadata: &str,
    #[case] kept: bool,
) {
    let wire = format!(r#"[{{"role":"user","content":"hi","metadata":{metadata}}}]"#);
    let converted = convert_messages(&messages(&wire));
    assert_eq!(converted[0].metadata.is_some(), kept);
    if kept {
        assert_eq!(
            serde_json::to_string(&converted[0]).unwrap(),
            r#"{"role":"human","content":"hi","metadata":{"skillId":"s"}}"#
        );
    }
}

#[test]
fn the_body_holds_the_assistant_id_and_the_converted_messages() {
    let body = body_text(
        r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#,
        false,
        r#"{"temperature":0.5}"#,
    );
    assert_eq!(
        body,
        r#"{"assistant_id":"agent","input":{"messages":[{"role":"system","content":"s"},{"role":"human","content":"u"}]}}"#
    );
}

#[rstest]
#[case::default_mode("{}", r#","stream_mode":"messages-tuple""#)]
#[case::custom_mode(r#"{"stream_mode":"values"}"#, r#","stream_mode":"values""#)]
#[case::null_mode(r#"{"stream_mode":null}"#, r#","stream_mode":null"#)]
fn streaming_adds_the_stream_mode(#[case] extra: &str, #[case] expected: &str) {
    let body = body_text(r#"[{"role":"user","content":"u"}]"#, true, extra);
    assert_eq!(
        body,
        format!(
            r#"{{"assistant_id":"agent","input":{{"messages":[{{"role":"human","content":"u"}}]}}{expected}}}"#
        )
    );
}

#[rstest]
#[case::no_options("{}")]
#[case::stream_option_alone(r#"{"stream":true}"#)]
#[case::mode_option_alone(r#"{"stream_mode":"values"}"#)]
fn a_request_without_the_litellm_params_stream_flag_has_no_stream_mode(#[case] extra: &str) {
    let body = body_text(r#"[{"role":"user","content":"u"}]"#, false, extra);
    assert!(!body.contains("stream_mode"), "{body}");
}

#[rstest]
#[case::config(
    r#"{"config":{"recursion_limit":5}}"#,
    r#","config":{"recursion_limit":5}"#
)]
#[case::metadata(r#"{"metadata":{"k":"v"}}"#, r#","metadata":{"k":"v"}"#)]
#[case::thread_id(r#"{"thread_id":"t-1"}"#, r#","thread_id":"t-1""#)]
#[case::null_config(r#"{"config":null}"#, r#","config":null"#)]
#[case::null_metadata(r#"{"metadata":null}"#, r#","metadata":null"#)]
#[case::null_thread_id(r#"{"thread_id":null}"#, r#","thread_id":null"#)]
fn config_metadata_and_thread_id_are_copied_when_present(
    #[case] extra: &str,
    #[case] expected: &str,
) {
    let body = body_text(r#"[{"role":"user","content":"u"}]"#, false, extra);
    assert_eq!(
        body,
        format!(
            r#"{{"assistant_id":"agent","input":{{"messages":[{{"role":"human","content":"u"}}]}}{expected}}}"#
        )
    );
}

#[rstest]
#[case::stream_mode(r#"{"stream_mode":3}"#, true, "stream_mode")]
#[case::config(r#"{"config":"x"}"#, false, "config")]
#[case::metadata(r#"{"metadata":[1]}"#, false, "metadata")]
fn an_option_of_the_wrong_type_is_rejected_by_name(
    #[case] extra: &str,
    #[case] stream: bool,
    #[case] field: &str,
) {
    let error = transform_request("agent", &[], &options(extra), stream).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the LangGraph option {field} has the wrong type")
    );
}

#[test]
fn a_stream_mode_of_the_wrong_type_is_ignored_without_streaming() {
    let request = transform_request("agent", &[], &options(r#"{"stream_mode":3}"#), false).unwrap();
    assert_eq!(request.stream_mode, None);
}

#[test]
fn the_error_class_keeps_the_status_and_message() {
    let error = get_error_class("boom", 502);
    assert_eq!(
        error,
        LangGraphError {
            status_code: 502,
            message: String::from("boom"),
        }
    );
    assert_eq!(error.to_string(), "boom");
}

#[rstest]
#[case::separators(r#"{"a":[1,2],"b":{"c":null}}"#, r#"{"a": [1, 2], "b": {"c": null}}"#)]
#[case::non_ascii(r#"{"t":"\u00e9\ud83d\ude00"}"#, r#"{"t": "\u00e9\ud83d\ude00"}"#)]
#[case::escapes(r#"["a\"b\\c\n"]"#, r#"["a\"b\\c\n"]"#)]
#[case::delete_is_escaped(r#"["\u007f"]"#, r#"["\u007f"]"#)]
fn dumps_matches_python_json_dumps(#[case] wire: &str, #[case] expected: &str) {
    let value: Json = serde_json::from_str(wire).unwrap();
    assert_eq!(dumps(&value).unwrap(), expected);
}

#[test]
fn the_unused_records_read_and_write_the_litellm_shapes() {
    let event: LangGraphStreamEvent =
        serde_json::from_str(r#"{"event":"values","data":1}"#).unwrap();
    assert_eq!(event.event.as_deref(), Some("values"));
    let response: LangGraphResponse = serde_json::from_str(
        r#"{"messages":[{"type":"ai","content":"x","id":"1","name":null}],"values":{}}"#,
    )
    .unwrap();
    assert_eq!(response.messages[0].kind.as_deref(), Some("ai"));
    let parsed = LangGraphParsedResponse {
        content: String::from("x"),
        role: String::from("assistant"),
        usage: Some(Usage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
        }),
    };
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        r#"{"content":"x","role":"assistant","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#
    );
}

struct WordCounter;

#[async_trait]
impl TokenCounter for WordCounter {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<u64, CountError> {
        assert_eq!(model, "gpt-3.5-turbo");
        u64::try_from(messages.len())
            .ok()
            .and_then(|count| count.checked_mul(10))
            .ok_or(CountError::Overflow)
    }

    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<u64, CountError> {
        assert_eq!(model, "gpt-3.5-turbo");
        u64::try_from(text.split_whitespace().count()).map_err(|_overflow| CountError::Overflow)
    }
}

fn warn_log<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let path = env::temp_dir().join(format!("betterlite-langgraph-chat-{}.log", process::id()));
    let log = fs::File::create(&path).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::WARN)
        .with_writer(Arc::new(log))
        .finish();
    let result = with_default(subscriber, run);
    let text = fs::read_to_string(&path).unwrap();
    fs::remove_file(&path).unwrap();
    (result, text)
}

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn run_reply(
    body: &str,
    status: u16,
    counter: &dyn TokenCounter,
) -> Result<LangGraphReply, LangGraphError> {
    let prompt = messages(r#"[{"role":"user","content":"hello"}]"#);
    let input = ReplyInput {
        model: "langgraph/agent",
        status,
        body: body.as_bytes(),
        messages: &prompt,
    };
    runtime().block_on(transform_response(&input, counter))
}

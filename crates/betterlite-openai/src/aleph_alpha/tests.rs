#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::adapter::{AlephAlphaAdapter, CallPhase, CompletionOutcome, mask_sensitive_info};
use super::config::{AlephAlphaConfig, ConfigError};
use super::error::AlephAlphaError;
use super::lines::{LineDecoder, decode_text};
use super::reply::{decode_aleph_reply, resolve_usage};
use super::request::{
    CompletionInput, CompletionRequest, build_prompt, completion_request, validate_environment,
};
use crate::chat::{ChatMessage, MessageContent, Role};
use crate::openai_base_config::Object;

fn message(role: Role, content: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(content.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

fn aleph_config(max_tokens: i64, overrides: &str) -> AlephAlphaConfig {
    AlephAlphaConfig::new(max_tokens, params(overrides)).unwrap()
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn cl100k() -> Engine {
    let data = assets::bundled(Encoding::Cl100kBase).unwrap();
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).unwrap();
    Engine::Tiktoken(Arc::new(engine))
}

fn count(tokenizer: &Engine, text: &str) -> u64 {
    u64::try_from(tokenizer.encode(text).unwrap().len()).unwrap()
}

fn request_for(
    messages: &[ChatMessage],
    config: &AlephAlphaConfig,
    optional: &Object,
) -> CompletionRequest {
    completion_request(&CompletionInput {
        model: "luminous-base-control",
        messages,
        api_base: "https://api.aleph-alpha.com/complete",
        api_key: None,
        optional_params: optional,
        config,
        created: 1_700_000_000,
    })
    .unwrap()
}

#[rstest]
#[case::first_assistant_becomes_instruction(
    "luminous-base-control",
    vec![(Role::Assistant, "A"), (Role::System, "S"), (Role::User, "U"), (Role::Assistant, "B")],
    "###Instruction: A###Instruction: S###Input: U###Response: B",
)]
#[case::later_roles_label_their_parts(
    "luminous-base-control",
    vec![(Role::User, "U"), (Role::Developer, "D"), (Role::Tool, "T"), (Role::Function, "F")],
    "###Instruction: U###Response: D###Response: T###Response: F",
)]
#[case::other_models_join_content_with_spaces(
    "luminous-base",
    vec![(Role::System, "S"), (Role::User, "U")],
    "S U",
)]
#[case::no_messages_give_an_empty_prompt("luminous-base", Vec::new(), "")]
fn control_roles(#[case] model: &str, #[case] given: Vec<(Role, &str)>, #[case] expected: &str) {
    let messages: Vec<ChatMessage> = given
        .into_iter()
        .map(|(role, content)| message(role, content))
        .collect();
    assert_eq!(build_prompt(model, &messages).unwrap(), expected);
}

#[rstest]
#[case::explicit_zero_wins(
    "{}",
    r#"{"maximum_tokens": 0, "temperature": 0, "echo": false}"#,
    r#"{"maximum_tokens": 0, "temperature": 0, "echo": false}"#
)]
#[case::explicit_zero_beats_a_config_value(
    r#"{"temperature": 7}"#,
    r#"{"temperature": 0}"#,
    r#"{"temperature": 0, "maximum_tokens": 4096}"#
)]
#[case::the_config_fills_only_missing_keys(
    r#"{"temperature": 7, "best_of": 0}"#,
    "{}",
    r#"{"temperature": 7, "best_of": 0, "maximum_tokens": 4096}"#
)]
#[case::maximum_tokens_starts_at_the_global_value("{}", "{}", r#"{"maximum_tokens": 4096}"#)]
fn config_zero_override(#[case] overrides: &str, #[case] optional: &str, #[case] expected: &str) {
    let config = aleph_config(4096, overrides);
    let messages = [message(Role::User, "Hi")];
    let request = request_for(&messages, &config, &params(optional));
    let wanted_value = value(expected);
    let wanted = wanted_value.as_object().unwrap();
    for (key, value) in wanted {
        assert_eq!(request.body.get(key), Some(value), "{key}");
    }
    assert_eq!(
        request.body.len(),
        wanted.len().saturating_add(2),
        "only model and prompt join the merged parameters"
    );
}

#[test]
fn config_rejects_a_setting_lite_llm_does_not_declare() {
    let error = AlephAlphaConfig::new(4096, params(r#"{"top_secret": 1}"#)).unwrap_err();
    assert!(matches!(error, ConfigError::UnknownSetting { key } if key == "top_secret"));
}

#[test]
fn the_config_snapshot_holds_only_the_non_null_defaults() {
    let config = aleph_config(1234, r#"{"temperature": null, "echo": true}"#);
    let mut merged = Object::new();
    config.merge_defaults(&mut merged);
    assert_eq!(merged, params(r#"{"maximum_tokens": 1234, "echo": true}"#));
}

#[rstest]
#[case::no_key(None, false)]
#[case::empty_key(Some(""), false)]
#[case::some_key(Some("sk-aleph"), true)]
fn validate_environment_adds_a_bearer_only_for_a_nonempty_key(
    #[case] given: Option<&str>,
    #[case] bearer: bool,
) {
    let key = given.map(SecretString::from);
    let headers = validate_environment(key.as_ref()).unwrap();
    assert_eq!(headers.get("accept").unwrap(), "application/json");
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    let authorization = headers
        .get("authorization")
        .map(|value| value.to_str().unwrap());
    if bearer {
        assert_eq!(authorization, Some("Bearer sk-aleph"));
    } else {
        assert_eq!(authorization, None);
    }
}

#[test]
fn a_message_that_is_not_plain_text_names_its_index() {
    let messages = [
        message(Role::User, "Hi"),
        ChatMessage {
            content: None,
            ..message(Role::User, "")
        },
    ];
    let error = build_prompt("luminous-base", &messages).unwrap_err();
    assert!(matches!(error, AlephAlphaError::Content { index: 1 }));
}

#[test]
fn the_endpoint_is_sent_unchanged() {
    let messages = [message(Role::User, "Hi")];
    let config = aleph_config(4096, "{}");
    let request = completion_request(&CompletionInput {
        model: "luminous-base",
        messages: &messages,
        api_base: "http://aleph.example:8080/custom/complete?x=1",
        api_key: None,
        optional_params: &Object::new(),
        config: &config,
        created: 1,
    })
    .unwrap();
    assert_eq!(
        request.url.as_str(),
        "http://aleph.example:8080/custom/complete?x=1"
    );
    assert!(!request.stream);
}

#[test]
fn multiple_choices() {
    let tokenizer = cl100k();
    let body = br#"{"completions":[{"completion":"one","finish_reason":"stop"},{"completion":"a much longer second completion","finish_reason":"length"},{"completion":"","finish_reason":"max_tokens"}]}"#;
    let choices = decode_aleph_reply(200, body).unwrap();
    let indices: Vec<u32> = choices.iter().map(|choice| choice.index).collect();
    assert_eq!(indices, vec![1, 2, 3]);
    let contents: Vec<Option<&str>> = choices
        .iter()
        .map(|choice| choice.message.content.as_deref())
        .collect();
    assert_eq!(
        contents,
        vec![Some("one"), Some("a much longer second completion"), None]
    );
    let reasons: Vec<Option<&str>> = choices
        .iter()
        .map(|choice| choice.finish_reason.as_deref())
        .collect();
    assert_eq!(reasons, vec![Some("stop"), Some("length"), Some("length")]);
    let native = choices[2]
        .provider_specific_fields
        .get("native_finish_reason")
        .and_then(Value::as_str);
    assert_eq!(native, Some("max_tokens"));
    let usage = resolve_usage("a b", &choices, &tokenizer).unwrap();
    assert_eq!(usage.prompt_tokens, count(&tokenizer, "a b"));
    assert_eq!(usage.completion_tokens, count(&tokenizer, "one"));
    assert_eq!(
        usage.total_tokens,
        count(&tokenizer, "a b").saturating_add(count(&tokenizer, "one"))
    );
}

#[rstest]
#[case::text_error(r#"{"error":"nope"}"#, 200, Some("nope"))]
#[case::text_error_keeps_the_http_status(
    r#"{"error":"invalid token"}"#,
    401,
    Some("invalid token")
)]
#[case::object_error(
    r#"{"error":{"message":"bad","code":500}}"#,
    500,
    Some(r#"{"message":"bad","code":500}"#)
)]
#[case::null_error(r#"{"error":null}"#, 200, Some("null"))]
#[case::missing_completions(r#"{"completions":7}"#, 200, None)]
#[case::missing_finish_reason(r#"{"completions":[{"completion":"x"}]}"#, 200, None)]
#[case::not_json("not a reply", 502, None)]
#[case::json_array("[]", 200, None)]
fn error_body(#[case] body: &str, #[case] status: u16, #[case] message: Option<&str>) {
    match decode_aleph_reply(status, body.as_bytes()) {
        Ok(_) => panic!("the reply {body:?} must not decode"),
        Err(AlephAlphaError::Provider {
            status_code,
            message: found,
        }) => {
            assert_eq!(status_code, status);
            assert_eq!(found, message.unwrap_or(body));
        }
        Err(other) => panic!("expected a provider error, got {other:?}"),
    }
}

#[test]
fn empty_completion() {
    let tokenizer = cl100k();
    let body = br#"{"completions":[{"completion":"a","finish_reason":"stop"},{"completion":"","finish_reason":"length"}]}"#;
    let choices = decode_aleph_reply(200, body).unwrap();
    assert_eq!(choices[1].message.content, None);
    assert_eq!(choices[1].finish_reason.as_deref(), Some("length"));
    let usage = resolve_usage("a b", &choices, &tokenizer).unwrap();
    assert_eq!(usage.completion_tokens, count(&tokenizer, "a"));
}

#[test]
fn an_empty_first_completion_counts_no_completion_tokens() {
    let tokenizer = cl100k();
    let body = br#"{"completions":[{"completion":"","finish_reason":"stop"}]}"#;
    let choices = decode_aleph_reply(200, body).unwrap();
    assert_eq!(choices[0].message.content, None);
    let usage = resolve_usage("a b", &choices, &tokenizer).unwrap();
    assert_eq!(usage.completion_tokens, 0);
}

#[test]
fn a_reply_without_choices_has_no_usage() {
    let choices = decode_aleph_reply(200, br#"{"completions":[]}"#).unwrap();
    assert!(matches!(
        resolve_usage("a", &choices, &cl100k()),
        Err(AlephAlphaError::NoChoices)
    ));
}

struct Captured {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: Value,
}

async fn serve(listener: TcpListener, status: u16, answer: &[u8]) -> Captured {
    let (mut stream, _) = listener.accept().await.unwrap();
    let captured = read_request(&mut stream).await;
    let head = format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        answer.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(answer).await.unwrap();
    stream.shutdown().await.unwrap();
    captured
}

async fn read_request(stream: &mut TcpStream) -> Captured {
    let mut raw = Vec::new();
    loop {
        if raw.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        raw.push(byte[0]);
    }
    let head = String::from_utf8(raw).unwrap();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .get("content-length")
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    Captured {
        request_line,
        headers,
        body: serde_json::from_slice(&body).unwrap(),
    }
}

fn adapter(tokenizer: Engine) -> AlephAlphaAdapter {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    AlephAlphaAdapter {
        client: ProxiedClient::outbound(&Arc::new(policy)).unwrap(),
        tokenizer,
        timeout: Duration::from_mins(1),
    }
}

fn input<'call>(
    base: &'call str,
    messages: &'call [ChatMessage],
    config: &'call AlephAlphaConfig,
    optional: &'call Object,
) -> CompletionInput<'call> {
    CompletionInput {
        model: "luminous-base",
        messages,
        api_base: base,
        api_key: None,
        optional_params: optional,
        config,
        created: 1_700_000_000,
    }
}

fn phase_name(phase: &CallPhase<'_>) -> &'static str {
    match phase {
        CallPhase::Pre { .. } => "pre_call",
        CallPhase::Post { .. } => "post_call",
    }
}

#[tokio::test]
async fn raw_stream_hands_lines_over_before_json_error_handling() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}/complete");
    let answer = b"{\"completions\":[{\"completion\":\"a\"}]}\r\n\r\n{\"error\":\"nope\"}\n";
    let messages = [message(Role::User, "Hi")];
    let config = aleph_config(4096, "{}");
    let optional = params(r#"{"stream": true}"#);
    let adapter = adapter(cl100k());
    let call = input(&base, &messages, &config, &optional);
    let mut phases = Vec::new();
    let mut sink = |phase: CallPhase<'_>| phases.push(phase_name(&phase));
    let (outcome, captured) = tokio::join!(
        adapter.completion(&call, &mut sink),
        serve(listener, 200, answer)
    );
    let CompletionOutcome::Stream(mut lines) = outcome.unwrap() else {
        panic!("stream=True must hand back the raw lines");
    };
    let mut collected = Vec::new();
    while let Some(line) = lines.next().await {
        collected.push(line.unwrap());
    }
    assert_eq!(
        collected,
        vec![
            String::from("{\"completions\":[{\"completion\":\"a\"}]}"),
            String::new(),
            String::from("{\"error\":\"nope\"}")
        ]
    );
    assert_eq!(phases, vec!["pre_call"]);
    assert_eq!(captured.request_line, "POST /complete HTTP/1.1");
    assert_eq!(captured.body.get("stream"), Some(&Value::Bool(true)));
    assert_eq!(captured.body.get("prompt"), Some(&Value::from("Hi")));
    assert_eq!(
        captured.headers.get("accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(captured.headers.get("authorization"), None);
}

type Call = (
    Result<CompletionOutcome, AlephAlphaError>,
    Vec<&'static str>,
);

async fn call_with(status: u16, answer: &[u8], options: &str, limit: Option<usize>) -> Call {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}/complete");
    let messages = [message(Role::User, "Hi")];
    let config = aleph_config(4096, "{}");
    let optional = params(options);
    let mut adapter = adapter(cl100k());
    if let Some(bytes) = limit {
        adapter.client.limits.body_bytes = bytes;
    }
    let call = input(&base, &messages, &config, &optional);
    let mut phases = Vec::new();
    let mut sink = |phase: CallPhase<'_>| phases.push(phase_name(&phase));
    let (outcome, _captured) = tokio::join!(
        adapter.completion(&call, &mut sink),
        serve(listener, status, answer)
    );
    (outcome, phases)
}

const COMPLETIONS: &[u8] = br#"{"completions":[{"completion":"a","finish_reason":"stop"}]}"#;

#[rstest]
#[case::stream_unauthorized(401, b"denied", r#"{"stream": true}"#)]
#[case::stream_server_error(500, COMPLETIONS, r#"{"stream": true}"#)]
#[case::buffered_server_error_with_completions(500, COMPLETIONS, "{}")]
#[case::buffered_not_found(404, b"gone", "{}")]
#[tokio::test]
async fn non_success_status_raises_before_streaming_or_parsing(
    #[case] status: u16,
    #[case] answer: &[u8],
    #[case] optional: &str,
) {
    let (outcome, phases) = call_with(status, answer, optional, None).await;
    let Err(AlephAlphaError::Provider {
        status_code,
        message,
    }) = outcome
    else {
        panic!("a non-2xx reply must raise AlephAlphaError::Provider");
    };
    assert_eq!(status_code, status);
    assert_eq!(message.as_bytes(), answer);
    assert_eq!(phases, vec!["pre_call"]);
}

#[rstest]
#[case::line_at_the_limit(b"12345678", &["Ok:12345678"])]
#[case::line_one_byte_over_the_limit(b"123456789", &["Err:a streamed line is longer than 8 bytes"])]
#[case::short_lines_past_the_limit_in_total(b"1234\n5678\n12", &["Ok:1234", "Ok:5678", "Ok:12"])]
#[case::lines_before_an_oversized_one(b"ab\n123456789", &["Ok:ab", "Err:a streamed line is longer than 8 bytes"])]
#[tokio::test]
async fn a_streamed_line_past_the_byte_limit_ends_the_stream(
    #[case] answer: &[u8],
    #[case] expected: &[&str],
) {
    let (outcome, _phases) = call_with(200, answer, r#"{"stream": true}"#, Some(8)).await;
    let Ok(CompletionOutcome::Stream(mut lines)) = outcome else {
        panic!("a literal true stream hands back the raw lines");
    };
    let mut seen = Vec::new();
    while let Some(line) = lines.next().await {
        seen.push(match line {
            Ok(text) => format!("Ok:{text}"),
            Err(error) => format!("Err:{error}"),
        });
    }
    assert_eq!(seen, expected);
}

#[rstest]
#[case::no_key("bad token", "bad token")]
#[case::key_to_the_end("url?key=sk-1", "url?key=[REDACTED_API_KEY]")]
#[case::key_before_another_parameter("url?key=sk-1&x=2", "url?key=[REDACTED_API_KEY]&x=2")]
#[case::only_the_first_key("key=a&key=b", "key=[REDACTED_API_KEY]&key=b")]
#[case::empty_value("key=", "key=[REDACTED_API_KEY]")]
fn error_text_masks_the_first_key_parameter(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(mask_sensitive_info(text), expected);
}

#[rstest]
#[case::plain(b"abc", "abc")]
#[case::invalid_byte_is_replaced(b"a\xffb", "a\u{fffd}b")]
#[case::cut_sequence_at_the_end_is_replaced(b"a\xe2\x82", "a\u{fffd}")]
#[case::complete_sequence_is_kept(b"\xe2\x82\xac", "\u{20ac}")]
#[case::bom_is_kept(b"\xef\xbb\xbfx", "\u{feff}x")]
fn decode_text_replaces_invalid_utf8(#[case] bytes: &[u8], #[case] expected: &str) {
    assert_eq!(decode_text(bytes), expected);
}

#[tokio::test]
async fn completion_counts_the_prompt_and_the_first_choice() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}/complete");
    let tokenizer = cl100k();
    let answer = br#"{"completions":[{"completion":"one","finish_reason":"stop"},{"completion":"two","finish_reason":"length"}]}"#;
    let messages = [message(Role::User, "Hi")];
    let config = aleph_config(1234, "{}");
    let optional = Object::new();
    let adapter = adapter(tokenizer.clone());
    let call = input(&base, &messages, &config, &optional);
    let mut phases = Vec::new();
    let mut sink = |phase: CallPhase<'_>| phases.push(phase_name(&phase));
    let (outcome, captured) = tokio::join!(
        adapter.completion(&call, &mut sink),
        serve(listener, 200, answer)
    );
    let CompletionOutcome::Reply(reply) = outcome.unwrap() else {
        panic!("a literal false stream decodes the buffered reply");
    };
    assert_eq!(reply.model, "luminous-base");
    assert_eq!(reply.created, 1_700_000_000);
    assert_eq!(reply.choices.len(), 2);
    assert_eq!(reply.usage.prompt_tokens, count(&tokenizer, "Hi"));
    assert_eq!(reply.usage.completion_tokens, count(&tokenizer, "one"));
    assert_eq!(phases, vec!["pre_call", "post_call"]);
    assert_eq!(
        captured.body.get("maximum_tokens"),
        Some(&Value::from(1234_i64))
    );
    assert_eq!(
        captured.body.get("model"),
        Some(&Value::from("luminous-base"))
    );
}

#[rstest]
fn replays_aleph_lines(#[files("fuzz/corpus/aleph_lines/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let (cut, body) = bytes.split_first().unwrap();
    let (head, tail) = body.split_at(usize::from(*cut).min(body.len()));
    let mut whole = LineDecoder::new(usize::MAX);
    let mut whole_lines = VecDeque::new();
    whole.push(body, &mut whole_lines).unwrap();
    whole.flush(&mut whole_lines);
    let mut split = LineDecoder::new(usize::MAX);
    let mut split_lines = VecDeque::new();
    split.push(head, &mut split_lines).unwrap();
    split.push(tail, &mut split_lines).unwrap();
    split.flush(&mut split_lines);
    assert_eq!(whole_lines, split_lines);
}

#[rstest]
#[case::cr_then_lf_across_chunks(&[b"a\r".as_slice(), b"\nb".as_slice()], &["a", "b"])]
#[case::cut_multibyte_across_chunks(&[b"\xe2\x82".as_slice(), b"\xac\n".as_slice()], &["\u{20ac}"])]
#[case::cut_sequence_never_finished(&[b"a\xe2\x82".as_slice()], &["a\u{fffd}"])]
#[case::lone_cr_flushes_an_empty_line(&[b"\r".as_slice()], &[""])]
#[case::unicode_separator(&[b"a\xe2\x80\xa8b\n".as_slice()], &["a", "b"])]
fn line_decoder_follows_httpx(#[case] chunks: &[&[u8]], #[case] expected: &[&str]) {
    let mut decoder = LineDecoder::new(usize::MAX);
    let mut lines = VecDeque::new();
    for chunk in chunks {
        decoder.push(chunk, &mut lines).unwrap();
    }
    decoder.flush(&mut lines);
    assert_eq!(lines, expected);
}

#[rstest]
fn replays_aleph_reply(#[files("fuzz/corpus/aleph_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _decoded = decode_aleph_reply(200, &bytes);
    let _failed = decode_aleph_reply(500, &bytes);
}

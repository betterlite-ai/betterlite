#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::shadow_unrelated,
        clippy::too_many_lines,
        reason = "each case carries several provider replies in one function and rebinds the reply names as it goes"
    )
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::runtime::runtime_counter;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::build::{RecoveredBody, Recovery, build_limit_response};
use super::tokens::{PromptCountError, PromptTokens};
use super::{
    ErrorBody, OUTPUT_TOKEN_LIMIT_MARKER, RejectedFieldsError, decode_rejected_fields,
    without_rejected_fields,
};

const COMPLETION: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"up"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

fn fields(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn header_map(entries: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in entries {
        map.insert(*name, HeaderValue::from_static(value));
    }
    map
}

// the prompt count is injected in most cases; the production tokenizer is asserted where the count itself matters
struct Recorder {
    calls: Mutex<Vec<(String, Vec<Value>)>>,
    tokens: usize,
}

impl Recorder {
    fn new(tokens: usize) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            tokens,
        }
    }

    fn calls(&self) -> Vec<(String, Vec<Value>)> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl PromptTokens for Recorder {
    async fn count(&self, model: &str, messages: &[Value]) -> Result<usize, PromptCountError> {
        self.calls
            .lock()
            .expect("calls")
            .push((model.to_owned(), messages.to_vec()));
        Ok(self.tokens)
    }
}

fn recovery(tokens: &dyn PromptTokens) -> Recovery<'_> {
    Recovery {
        clock: fixed_clock,
        next_id: fixed_id,
        tokens,
    }
}

fn two_element_loc() {
    let body = json!({"message": "{\"detail\": [{\"loc\": [\"body\", \"temperature\"]}, {\"loc\": [\"body\", \"top_p\"]}, {\"loc\": [\"body\"]}, {\"loc\": []}, {\"loc\": [\"body\", \"seed\", \"extra\"]}, {\"loc\": \"body\"}, {\"loc\": null}, {}]}"});
    let data = fields(&[
        ("temperature", json!(1)),
        ("top_p", json!(2)),
        ("seed", json!(3)),
    ]);
    let dropped = without_rejected_fields(ErrorBody::Sdk(&body), &data).expect("sdk body");
    assert_eq!(dropped, fields(&[("seed", json!(3))]));
    assert_eq!(data.len(), 3, "the caller's map is untouched");
    let http = json!({"error": {"message": "{\"detail\": [{\"loc\": [\"body\", \"seed\"]}]}"}});
    let text = serde_json::to_vec(&http).expect("http body");
    let dropped = without_rejected_fields(ErrorBody::Http(&text), &data).expect("http");
    assert_eq!(
        dropped,
        fields(&[("temperature", json!(1)), ("top_p", json!(2))])
    );
    let direct = json!({"message": {"detail": [{"loc": ["body", "top_p"]}]}});
    let dropped = without_rejected_fields(ErrorBody::Sdk(&direct), &data).expect("direct");
    assert_eq!(
        dropped,
        fields(&[("temperature", json!(1)), ("seed", json!(3))])
    );
    let unparsable = json!({"message": "not json at all"});
    let dropped =
        without_rejected_fields(ErrorBody::Sdk(&unparsable), &data).expect("unparsable message");
    assert_eq!(dropped, data);
}

fn malformed_detail_keeps_fields() {
    let data = fields(&[("temperature", json!(1)), ("seed", json!(3))]);
    let kept = [
        json!(null),
        json!("a string body"),
        json!({"message": ""}),
        json!({"message": 0}),
        json!({"message": []}),
        json!({"message": {}}),
        json!({"message": "{}"}),
        json!({"message": "{\"detail\": []}"}),
        json!({"message": "{\"detail\": {\"loc\": [\"body\", \"temperature\"]}}"}),
        json!({"message": "{\"detail\": [5, {\"loc\": [\"body\", \"temperature\"]}]}"}),
    ];
    for body in kept {
        let dropped = without_rejected_fields(ErrorBody::Sdk(&body), &data).expect("kept fields");
        assert_eq!(dropped, data, "{body}");
    }
    let dropped = without_rejected_fields(ErrorBody::Http(b"{}"), &data).expect("no error key");
    assert_eq!(dropped, data);
}

// the truthiness the message and loc checks read; an empty value is absent even though the field is there
#[test]
fn present_matches_python_truthiness() {
    assert!(!super::present(&json!(null)));
    assert!(!super::present(&json!(false)));
    assert!(!super::present(&json!(0)));
    assert!(!super::present(&json!("")));
    assert!(!super::present(&json!([])));
    assert!(!super::present(&json!({})));
    assert!(super::present(&json!(true)));
    assert!(super::present(&json!(1)));
    assert!(super::present(&json!(-1)));
    assert!(super::present(&json!("x")));
    assert!(super::present(&json!([0])));
    assert!(super::present(&json!({"a": 1})));
}

fn output_limit_case_insensitive() {
    let marker = OUTPUT_TOKEN_LIMIT_MARKER.to_uppercase();
    let rendered = format!("Error code: 400 - {{\"error\": {{\"message\": \"{marker}\"}}}}");
    assert!(ErrorBody::Http(rendered.as_bytes()).reaches_output_limit());
    let rendered = format!("Azure rejected gpt-5.1 with: {marker} for this request");
    assert!(ErrorBody::Http(rendered.as_bytes()).reaches_output_limit());
    for shorter in [
        "could not finish the message",
        "max_tokens or model output limit was reached",
        "Error code: 400 - invalid max_tokens value",
        "",
    ] {
        assert!(
            !ErrorBody::Http(shorter.as_bytes()).reaches_output_limit(),
            "{shorter}"
        );
    }
}

async fn empty_length_completion() {
    let recorder = Recorder::new(7);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        (
            "messages",
            json!([{"role": "user", "content": "hello world"}]),
        ),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("recovered reply");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    let RecoveredBody::Completion(completion) = answer.body else {
        panic!("a buffered request recovers a completion");
    };
    assert_eq!(
        serde_json::to_value(&completion).expect("completion json"),
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "provider_specific_fields": {},
                "message": {
                    "role": "assistant",
                    "content": "",
                    "provider_specific_fields": {"refusal": null},
                },
            }],
            "usage": {"prompt_tokens": 7, "completion_tokens": 0, "total_tokens": 7},
        })
    );
    assert_eq!(
        recorder.calls(),
        vec![(
            String::from("gpt-3.5-turbo"),
            vec![json!({"role": "user", "content": "hello world"})],
        )]
    );
}

async fn single_length_stream_chunk() {
    let recorder = Recorder::new(5);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", json!([{"role": "user", "content": "hi"}])),
        ("stream", json!(true)),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("recovered chunk");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    let RecoveredBody::Chunk(chunk) = answer.body else {
        panic!("a stream request recovers one chunk");
    };
    let serialized = serde_json::to_value(&chunk).expect("chunk json");
    assert_eq!(
        serialized,
        json!({
            "id": "chatcmpl-00000000-0000-0000-0000-000000000007",
            "object": "chat.completion.chunk",
            "created": 1_700_000_000,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "finish_reason": "length",
                "delta": {"role": "assistant", "content": ""},
            }],
        })
    );
    assert!(serialized.get("usage").is_none());
    assert_eq!(recorder.calls().len(), 1, "the prompt is counted anyway");
}

async fn prompt_token_count() {
    let recorder = Recorder::new(7);
    let messages = json!([{"role": "user", "content": "hello world"}]);
    let data = fields(&[
        ("model", json!("gpt-3.5-turbo")),
        ("messages", messages.clone()),
    ]);
    let answer = build_limit_response(
        &data,
        header_map(&[("x-request-id", "r1")]),
        &recovery(&recorder),
    )
    .await
    .expect("counted reply");
    assert_eq!(answer.headers, header_map(&[("x-request-id", "r1")]));
    assert_eq!(
        recorder.calls(),
        vec![(
            String::from("gpt-3.5-turbo"),
            messages.as_array().expect("messages").clone()
        )]
    );
    let unmodelled = fields(&[("messages", json!([]))]);
    build_limit_response(&unmodelled, HeaderMap::new(), &recovery(&recorder))
        .await
        .expect("empty model");
    assert_eq!(
        recorder.calls().last().map(|call| call.0.as_str()),
        Some("")
    );
    let uncounted = Recorder::new(7);
    for data in [
        fields(&[("model", json!("gpt-3.5-turbo"))]),
        fields(&[
            ("model", json!("gpt-3.5-turbo")),
            ("messages", json!("not a list")),
        ]),
    ] {
        let answer = build_limit_response(&data, HeaderMap::new(), &recovery(&uncounted))
            .await
            .expect("uncounted reply");
        let RecoveredBody::Completion(completion) = answer.body else {
            panic!("a buffered request recovers a completion");
        };
        assert_eq!(completion.usage.prompt_tokens, 0);
        assert_eq!(completion.usage.total_tokens, 0);
    }
    assert!(uncounted.calls().is_empty());
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("")
    .expect("config");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy");
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).expect("client"));
    let tokens =
        runtime_counter(&config, &client, &BundledCatalog::default().get()).expect("counter");
    let counted = build_limit_response(
        &fields(&[
            ("model", json!("gpt-3.5-turbo")),
            (
                "messages",
                json!([{"role": "user", "content": "hello world"}]),
            ),
        ]),
        HeaderMap::new(),
        &Recovery {
            clock: fixed_clock,
            next_id: fixed_id,
            tokens: &tokens,
        },
    )
    .await;
    let RecoveredBody::Completion(completion) = counted.expect("tokenized reply").body else {
        panic!("a buffered request recovers a completion");
    };
    assert_eq!(completion.usage.prompt_tokens, 9);
    assert_eq!(completion.usage.total_tokens, 9);
}

fn malformed_nested_error_failure() {
    let number = json!({"message": "42"});
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Sdk(&number)),
        Err(RejectedFieldsError::MessageNotObject)
    ));
    let later_scalar = json!({"message": "{\"detail\": [{\"loc\": [\"body\", \"a\"]}, 5]}"});
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Sdk(&later_scalar)),
        Err(RejectedFieldsError::DetailEntryNotObject { index: 1 })
    ));
    let listed = json!({"error": {"message": "[1]"}});
    let listed = serde_json::to_vec(&listed).expect("http body");
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(&listed)),
        Err(RejectedFieldsError::MessageNotObject)
    ));
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(b"not json")),
        Err(RejectedFieldsError::Json { .. })
    ));
    assert!(matches!(
        decode_rejected_fields(ErrorBody::Http(b"[1, 2]")),
        Err(RejectedFieldsError::ResponseNotObject)
    ));
}

mod raw_send;

#[derive(Clone, Copy)]
enum Scenario {
    TwoElementLoc,
    MalformedDetailKeepsFields,
    OutputLimitCaseInsensitive,
    EmptyLengthCompletion,
    SingleLengthStreamChunk,
    PromptTokenCount,
    UnrelatedError,
    MalformedNestedErrorFailure,
    AzureRawSendHeaders,
    AzureOutputLimitRecovery,
    AzureAsyncTimeout,
}

#[rstest]
#[case::two_element_loc(Scenario::TwoElementLoc)]
#[case::malformed_detail_keeps_fields(Scenario::MalformedDetailKeepsFields)]
#[case::output_limit_case_insensitive(Scenario::OutputLimitCaseInsensitive)]
#[case::empty_length_completion(Scenario::EmptyLengthCompletion)]
#[case::single_length_stream_chunk(Scenario::SingleLengthStreamChunk)]
#[case::prompt_token_count(Scenario::PromptTokenCount)]
#[case::unrelated_error(Scenario::UnrelatedError)]
#[case::malformed_nested_error_failure(Scenario::MalformedNestedErrorFailure)]
#[case::azure_raw_send_headers(Scenario::AzureRawSendHeaders)]
#[case::azure_output_limit_recovery(Scenario::AzureOutputLimitRecovery)]
#[case::azure_async_timeout(Scenario::AzureAsyncTimeout)]
#[tokio::test]
async fn output_limits_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::TwoElementLoc => two_element_loc(),
        Scenario::MalformedDetailKeepsFields => malformed_detail_keeps_fields(),
        Scenario::OutputLimitCaseInsensitive => output_limit_case_insensitive(),
        Scenario::EmptyLengthCompletion => empty_length_completion().await,
        Scenario::SingleLengthStreamChunk => single_length_stream_chunk().await,
        Scenario::PromptTokenCount => prompt_token_count().await,
        Scenario::UnrelatedError => raw_send::unrelated_error().await,
        Scenario::MalformedNestedErrorFailure => malformed_nested_error_failure(),
        Scenario::AzureRawSendHeaders => raw_send::azure_raw_send_headers().await,
        Scenario::AzureOutputLimitRecovery => raw_send::azure_output_limit_recovery().await,
        Scenario::AzureAsyncTimeout => raw_send::azure_async_timeout().await,
    }
}

fn replays(path: &Path) -> bool {
    let bytes = std::fs::read(path).expect("corpus seed");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("seed name")
        .to_owned();
    let decoded = decode_rejected_fields(ErrorBody::Http(&bytes));
    match name.split('-').next().expect("seed category") {
        "valid" | "boundary" => decoded.is_ok(),
        "malformed" | "truncated" => decoded.is_err(),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_output_limits_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_output_limits");
    let seeds: Vec<PathBuf> = std::fs::read_dir(corpus)
        .expect("corpus directory")
        .map(|entry| entry.expect("seed").path())
        .filter(|path| path.is_file())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}

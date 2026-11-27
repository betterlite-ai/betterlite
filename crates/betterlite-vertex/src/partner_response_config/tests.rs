#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_anthropic::usage::ReasoningTokenCounter;
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
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde::Deserialize;
use uuid::Uuid;

use super::error::PartnerResponseError;
use super::{
    PartnerResponseCall, PartnerResponseConfig, PartnerResponseInput, partner_response_config,
};
use crate::vertex_relay_metadata::types::VertexPublisher;

const LOGGING_INIT: &str = r#"{"model":"jamba","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const LOGGING_NOW: &str = "2026-01-01T00:00:05+00:00";
const STARTING_ID: &str = "chatcmpl-b10159";
const NOW: i64 = 1_760_000_000;

const CLAUDE_BODY: &str = r#"{"model":"claude-3-sonnet-20240229","content":[{"type":"text","text":"world"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#;
const CLAUDE_THINKING_BODY: &str = r#"{"model":"claude-3-sonnet-20240229","content":[{"type":"thinking","thinking":"think"},{"type":"text","text":"world"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":10}}"#;
const AI21_CHOICES: &str = r#""choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]"#;
const AI21_USAGE: &str = r#","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#;
const LLAMA_CHOICES: &str = r#""choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]"#;

struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(7)
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<LogRecord>>);

impl LogSink for RecordingSink {
    fn emit(&self, record: LogRecord) {
        self.0.lock().expect("records").push(record);
    }
}

impl RecordingSink {
    fn records(&self) -> Vec<LogRecord> {
        self.0.lock().expect("records").clone()
    }
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init = serde_json::from_str::<LoggingInit>(LOGGING_INIT).expect("logging init");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("logging state")
}

fn logging_env<'env>(redactor: &'env Redactor, sink: &'env RecordingSink) -> PrePostEnv<'env> {
    let now = DateTime::<FixedOffset>::parse_from_rfc3339(LOGGING_NOW).expect("clock");
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

fn header_map(values: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }
    headers
}

struct Sample<'sample> {
    publisher: VertexPublisher,
    model: &'sample str,
    status: u16,
    body: &'sample str,
    headers: &'sample [(&'sample str, &'sample str)],
    now: i64,
}

struct Outcome {
    answer: Result<Json, PartnerResponseError<Infallible>>,
    records: Vec<LogRecord>,
}

fn transform(sample: &Sample<'_>) -> Outcome {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let sink = RecordingSink::default();
    let mut state = logging_state(&redactor);
    let env = logging_env(&redactor, &sink);
    let messages: Vec<Json> = Vec::new();
    let request_data = IndexMap::new();
    let optional_params = IndexMap::new();
    let litellm_params = IndexMap::new();
    let headers = header_map(sample.headers);
    let input = PartnerResponseInput {
        model: sample.model,
        starting_id: STARTING_ID,
        status: sample.status,
        headers: &headers,
        body: sample.body.as_bytes(),
        messages: &messages,
        request_data: &request_data,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        json_mode: None,
        now: sample.now,
        identifier: Uuid::nil,
        disable_add_prefix_to_prompt: false,
        state: &mut state,
        env: &env,
    };
    let answer =
        partner_response_config(sample.publisher).transform_response(PartnerResponseCall {
            input,
            counter: &FixedCounter,
        });
    Outcome {
        answer,
        records: sink.records(),
    }
}

#[derive(Debug, Deserialize)]
struct AnswerView {
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    choices: Vec<ChoiceView>,
    #[serde(default)]
    usage: Option<UsageView>,
}

#[derive(Debug, Deserialize)]
struct ChoiceView {
    #[serde(default)]
    index: i64,
    #[serde(default)]
    finish_reason: Option<String>,
    message: MessageView,
}

#[derive(Debug, Deserialize)]
struct MessageView {
    role: String,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageView {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    #[serde(default)]
    completion_tokens_details: Option<CompletionDetailsView>,
}

#[derive(Debug, Deserialize)]
struct CompletionDetailsView {
    #[serde(default)]
    reasoning_tokens: Option<i64>,
    #[serde(default)]
    text_tokens: Option<i64>,
}

fn answer_view(outcome: &Outcome) -> AnswerView {
    let answer = outcome.answer.as_ref().expect("the reply transforms");
    serde_json::from_str(&serde_json::to_string(answer).expect("answer JSON")).expect("answer view")
}

fn ai21_body(id: &str, created: &str, model: &str, usage: &str) -> String {
    format!("{{{id}{created}{model}{AI21_CHOICES}{usage}}}")
}

#[test]
fn select_anthropic_gives_claude() {
    assert_eq!(
        partner_response_config(VertexPublisher::Anthropic),
        PartnerResponseConfig::Claude
    );
}

#[test]
fn select_ai21_gives_ai21() {
    assert!(matches!(
        partner_response_config(VertexPublisher::Ai21),
        PartnerResponseConfig::Ai21(_)
    ));
}

#[test]
fn select_openapi_gives_llama3() {
    assert!(matches!(
        partner_response_config(VertexPublisher::OpenApi),
        PartnerResponseConfig::Llama3(_)
    ));
}

#[test]
fn select_mistralai_gives_llama3() {
    assert!(matches!(
        partner_response_config(VertexPublisher::MistralAi),
        PartnerResponseConfig::Llama3(_)
    ));
}

#[test]
fn transform_claude_names_the_requested_model() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Anthropic,
        model: "claude-sonnet-4-5",
        status: 200,
        body: CLAUDE_BODY,
        headers: &[],
        now: NOW,
    });
    let view = answer_view(&outcome);
    assert_eq!(view.id, STARTING_ID);
    assert_eq!(view.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(view.created, Some(NOW));
    assert_eq!(view.choices[0].index, 0);
    assert_eq!(view.choices[0].message.content.as_deref(), Some("world"));
    let usage = view.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 1);
    assert_eq!(usage.total_tokens, 4);
}

#[test]
fn transform_claude_counts_reasoning_with_the_given_counter() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Anthropic,
        model: "claude-sonnet-4-5",
        status: 200,
        body: CLAUDE_THINKING_BODY,
        headers: &[],
        now: NOW,
    });
    let usage = answer_view(&outcome).usage.expect("usage");
    let details = usage.completion_tokens_details.expect("details");
    assert_eq!(details.reasoning_tokens, Some(7));
    assert_eq!(details.text_tokens, Some(3));
}

#[test]
fn transform_claude_logs_the_raw_body() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Anthropic,
        model: "claude-sonnet-4-5",
        status: 200,
        body: CLAUDE_BODY,
        headers: &[],
        now: NOW,
    });
    assert_eq!(outcome.records.len(), 1);
    assert!(outcome.records[0].message.contains("world"));
}

#[test]
fn transform_claude_not_json_is_an_anthropic_error() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Anthropic,
        model: "claude-sonnet-4-5",
        status: 400,
        body: "not json",
        headers: &[],
        now: NOW,
    });
    let error = outcome.answer.expect_err("the body is not JSON");
    assert!(matches!(&error, PartnerResponseError::Claude(_)));
    assert_eq!(error.status_code(), Some(400));
}

#[test]
fn transform_ai21_keeps_the_reply_id_and_model() {
    let body = ai21_body(
        r#""id":"reply-1","#,
        r#""created":7,"#,
        r#""model":"jamba-1.5-mini","#,
        AI21_USAGE,
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    let view = answer_view(&outcome);
    assert_eq!(view.id, "reply-1");
    assert_eq!(view.model.as_deref(), Some("jamba-1.5-mini"));
    assert_eq!(view.created, Some(7));
    assert_eq!(view.choices[0].message.content.as_deref(), Some("hi"));
    assert_eq!(view.choices[0].finish_reason.as_deref(), Some("stop"));
    let usage = view.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 1);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 3);
    assert_eq!(outcome.records.len(), 1);
}

#[test]
fn transform_ai21_uses_the_starting_id_when_the_reply_id_is_missing() {
    let body = ai21_body("", r#""created":7,"#, r#""model":"jamba-1.5","#, AI21_USAGE);
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    assert_eq!(answer_view(&outcome).id, STARTING_ID);
}

#[test]
fn transform_ai21_uses_the_starting_id_when_the_reply_id_is_null() {
    let body = ai21_body(
        r#""id":null,"#,
        r#""created":7,"#,
        r#""model":"jamba-1.5","#,
        AI21_USAGE,
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    assert_eq!(answer_view(&outcome).id, STARTING_ID);
}

#[test]
fn transform_ai21_uses_the_starting_id_when_the_reply_id_is_empty() {
    let body = ai21_body(
        r#""id":"","#,
        r#""created":7,"#,
        r#""model":"jamba-1.5","#,
        AI21_USAGE,
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    assert_eq!(answer_view(&outcome).id, STARTING_ID);
}

#[test]
fn transform_ai21_reads_the_clock_when_created_is_missing() {
    let body = ai21_body(
        r#""id":"reply-1","#,
        "",
        r#""model":"jamba-1.5","#,
        AI21_USAGE,
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    assert_eq!(answer_view(&outcome).created, Some(NOW));
}

#[test]
fn transform_ai21_answers_zero_usage_when_the_reply_lacks_it() {
    let body = ai21_body(
        r#""id":"reply-1","#,
        r#""created":7,"#,
        r#""model":"jamba-1.5","#,
        "",
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    let usage = answer_view(&outcome).usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
}

#[test]
fn transform_ai21_answers_an_empty_model_when_the_reply_lacks_one() {
    let body = ai21_body(r#""id":"reply-1","#, r#""created":7,"#, "", AI21_USAGE);
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    assert_eq!(answer_view(&outcome).model.as_deref(), Some(""));
}

#[test]
fn transform_ai21_not_json_is_an_ai21_error() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::Ai21,
        model: "jamba-1.5",
        status: 502,
        body: "not json",
        headers: &[],
        now: NOW,
    });
    let error = outcome.answer.expect_err("the body is not JSON");
    assert!(matches!(&error, PartnerResponseError::Ai21(_)));
    assert_eq!(error.status_code(), Some(502));
}

#[test]
fn transform_llama_fills_missing_fields() {
    let body = format!("{{{LLAMA_CHOICES}}}");
    let outcome = transform(&Sample {
        publisher: VertexPublisher::OpenApi,
        model: "meta/llama-3.1-405b-instruct-maas",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    let view = answer_view(&outcome);
    assert_eq!(view.id, "");
    assert_eq!(view.created, Some(0));
    assert_eq!(
        view.model.as_deref(),
        Some("meta/llama-3.1-405b-instruct-maas")
    );
    assert_eq!(view.choices[0].message.role, "assistant");
    assert_eq!(view.choices[0].message.content.as_deref(), Some("hi"));
    let usage = view.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 0);
    assert_eq!(usage.completion_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
}

#[test]
fn transform_llama_keeps_the_reply_fields() {
    let body = format!(
        r#"{{"id":"l-1","created":5,"model":"upstream-model",{LLAMA_CHOICES}{AI21_USAGE}}}"#
    );
    let outcome = transform(&Sample {
        publisher: VertexPublisher::MistralAi,
        model: "meta/llama-3.1-405b-instruct-maas",
        status: 200,
        body: &body,
        headers: &[],
        now: NOW,
    });
    let view = answer_view(&outcome);
    assert_eq!(view.id, "l-1");
    assert_eq!(view.created, Some(5));
    assert_eq!(view.model.as_deref(), Some("upstream-model"));
    let usage = view.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 1);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 3);
    assert_eq!(outcome.records.len(), 1);
}

#[test]
fn transform_llama_not_json_is_a_vertex_error() {
    let outcome = transform(&Sample {
        publisher: VertexPublisher::OpenApi,
        model: "meta/llama-3.1-405b-instruct-maas",
        status: 503,
        body: "not json",
        headers: &[("x-request-id", "req-b10159")],
        now: NOW,
    });
    let failure = outcome.answer.expect_err("the body is not JSON");
    let PartnerResponseError::Llama(error) = failure else {
        panic!("a non-JSON Llama reply is a VertexAIError");
    };
    assert_eq!(error.status_code, 503);
    assert_eq!(
        error.headers,
        Some(BTreeMap::from([(
            String::from("x-request-id"),
            String::from("req-b10159")
        )]))
    );
}

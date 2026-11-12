#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::call::{ImageGenerationCall, prepare_image_call};
use super::decode::decode_image_reply;
use super::defaults::apply_family_defaults;
use super::error::{ErrorStatus, ImageGenerationFailure, ImageGenerationSource, OpenAiImageError};
use super::family::{
    DALL_E_2_PARAMS, DALL_E_3_PARAMS, GPT_IMAGE_PARAMS, ImageFamily, ImageParamError,
    map_openai_params, select_image_family,
};
use super::logging::PrePostLogger;
use super::reply::{ImageGenerationReply, ImageReplyError};
use super::request::{
    ImageGenerationOutcome, begin_image_generation, finish_image_generation, send_image_generation,
};
use super::transform::{ImageTransformCall, transform_image_reply};
use super::transport::ImageGenerationTransport;
use super::wire::outbound_request;
use crate::openai_base_config::Headers;

const INIT: &str = r#"{"model":"dall-e-2","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const API_BASE: &str = "https://example.test/v1";
const NOW: i64 = 1_700_000_000;

#[derive(Default)]
struct Sink(Mutex<Vec<LogRecord>>);

impl LogSink for Sink {
    fn emit(&self, record: LogRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

fn logging() -> (Redactor, LoggingState, Sink) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    (redactor, state, Sink::default())
}

fn logging_env<'setup>(
    redactor: &'setup Redactor,
    sink: &'setup Sink,
    logger: Option<LoggerFn<'setup>>,
) -> PrePostEnv<'setup> {
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
        logger_fn: logger,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    }
}

struct Runner {
    redactor: Redactor,
    state: LoggingState,
    sink: Sink,
}

impl Runner {
    fn new() -> Self {
        let (redactor, state, sink) = logging();
        Self {
            redactor,
            state,
            sink,
        }
    }

    async fn run<T>(
        &mut self,
        call: ImageGenerationCall,
        transport: &T,
    ) -> Result<ImageGenerationOutcome, ImageGenerationFailure>
    where
        T: ImageGenerationTransport,
    {
        let prepared = {
            let env = logging_env(&self.redactor, &self.sink, None);
            let mut logger = PrePostLogger {
                state: &mut self.state,
                env: &env,
            };
            begin_image_generation(call, &mut logger)?
        };
        let sent = send_image_generation(&prepared, transport).await;
        let env = logging_env(&self.redactor, &self.sink, None);
        let mut logger = PrePostLogger {
            state: &mut self.state,
            env: &env,
        };
        finish_image_generation(&prepared, sent, &mut logger)
    }
}

pub(super) fn options(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

fn decoded(body: &str) -> ImageGenerationReply {
    let bytes = body.as_bytes();
    decode_image_reply(bytes, NOW).unwrap()
}

pub(super) fn call(model: &str, optional_params: IndexMap<String, Json>) -> ImageGenerationCall {
    ImageGenerationCall {
        model: model.to_owned(),
        prompt: String::from("a cat"),
        optional_params,
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some(API_BASE.to_owned()),
        organization: Some(String::from("org-test")),
        headers: None,
        timeout: Duration::from_secs(30),
        delegate_async: false,
        now_seconds: NOW,
    }
}

#[derive(Clone)]
pub(super) struct Recorded {
    pub(super) body: Json,
    deadline: Duration,
    url: String,
    pub(super) query: Vec<(String, String)>,
    headers: HeaderMap,
}

pub(super) struct FakeTransport {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    failure: bool,
    requests: Mutex<Vec<Recorded>>,
}

impl FakeTransport {
    fn answering(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| (String::from(*name), String::from(*value)))
                .collect(),
            body: body.as_bytes().to_vec(),
            failure: false,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            status: 0,
            headers: BTreeMap::new(),
            body: Vec::new(),
            failure: true,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageGenerationTransport for FakeTransport {
    fn generate(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> {
        let body = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice::<Json>(bytes).unwrap(),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Json::Null
            }
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                body,
                deadline: request.deadline,
                url: request.url.to_string(),
                query: request.query,
                headers: request.headers,
            });
        let outcome = if self.failure {
            Err(TransportError::Closed)
        } else {
            let mut headers = HeaderMap::new();
            for (name, value) in &self.headers {
                headers.insert(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value).unwrap(),
                );
            }
            Ok(UpstreamResponse {
                status: self.status,
                headers,
                body: self.body.clone(),
            })
        };
        ready(outcome)
    }
}

#[rstest]
#[case::dalle2_parameters(0)]
#[case::dalle3_quality_style(1)]
#[case::gpt_image_usage(2)]
#[case::b64_json_and_url(3)]
#[case::missing_created(4)]
#[case::image_status_error(5)]
#[case::unsupported_image_param(6)]
#[case::image_request_overlay(7)]
#[case::image_raw_response_headers(8)]
#[tokio::test]
async fn image_generation_cases(#[case] case: u8) {
    match case {
        0 => dalle2_parameters(),
        1 => dalle3_quality_style(),
        2 => gpt_image_usage(),
        3 => b64_json_and_url(),
        4 => missing_created(),
        5 => image_status_error().await,
        6 => unsupported_image_param(),
        7 => image_request_overlay(),
        8 => image_raw_response_headers().await,
        _ => panic!("unknown case {case}"),
    }
}

fn dalle2_parameters() {
    assert_eq!(select_image_family(""), ImageFamily::DallE2);
    assert_eq!(select_image_family("dall-e-2"), ImageFamily::DallE2);
    assert_eq!(select_image_family("dall-e-2-preview"), ImageFamily::DallE2);
    assert_eq!(ImageFamily::DallE2.supported_params(), &DALL_E_2_PARAMS);
    let body = r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#;
    let mut defaulted = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut defaulted, &options("{}")).unwrap();
    assert_eq!(defaulted.size.as_deref(), Some("1024x1024"));
    assert_eq!(defaulted.quality.as_deref(), Some("standard"));
    assert_eq!(defaulted.output_format.as_deref(), Some("png"));
    let supplied = options(r#"{"size":"512x512","quality":"low"}"#);
    let mut kept = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut kept, &supplied).unwrap();
    assert_eq!(kept.size.as_deref(), Some("512x512"));
    assert_eq!(kept.quality.as_deref(), Some("low"));
    assert_eq!(kept.output_format.as_deref(), Some("png"));
    let explicit_null = options(r#"{"quality":null}"#);
    let mut nulled = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut nulled, &explicit_null).unwrap();
    assert_eq!(nulled.quality, None);
}

fn dalle3_quality_style() {
    assert_eq!(select_image_family("dall-e-3"), ImageFamily::DallE3);
    assert_eq!(select_image_family("dall-e-3-preview"), ImageFamily::DallE3);
    assert_eq!(ImageFamily::DallE3.supported_params(), &DALL_E_3_PARAMS);
    assert!(ImageFamily::DallE3.supported_params().contains(&"style"));
    let mut mapped = IndexMap::new();
    let requested = options(r#"{"style":"vivid"}"#);
    map_openai_params(
        ImageFamily::DallE3,
        "dall-e-3",
        &requested,
        &mut mapped,
        false,
    )
    .unwrap();
    assert_eq!(mapped.get("style"), requested.get("style"));
    let body = r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#;
    let mut defaulted = decoded(body);
    apply_family_defaults(ImageFamily::DallE3, &mut defaulted, &options("{}")).unwrap();
    assert_eq!(defaulted.size.as_deref(), Some("1024x1024"));
    assert_eq!(defaulted.quality.as_deref(), Some("hd"));
    assert_eq!(defaulted.output_format.as_deref(), Some("png"));
}

fn gpt_image_usage() {
    assert_eq!(select_image_family("gpt-image-1"), ImageFamily::GptImage);
    // the selector is case-sensitive, so an upper-case DALL-E name is a GPT image
    assert_eq!(select_image_family("DALL-E-3"), ImageFamily::GptImage);
    assert_eq!(ImageFamily::GptImage.supported_params(), &GPT_IMAGE_PARAMS);
    gpt_reply_values();
    gpt_option_values();
}

fn gpt_reply_values() {
    let body = r#"{"created":7,"data":[],"size":"1792x1024","quality":"medium","output_format":"jpeg","usage":{"input_tokens":4,"input_tokens_details":{"image_tokens":1,"text_tokens":3},"output_tokens":8,"total_tokens":12}}"#;
    let mut reply = decoded(body);
    let supplied = options(r#"{"size":"512x512","quality":"low"}"#);
    apply_family_defaults(ImageFamily::GptImage, &mut reply, &supplied).unwrap();
    assert_eq!(reply.size.as_deref(), Some("1792x1024"));
    assert_eq!(reply.quality.as_deref(), Some("medium"));
    assert_eq!(reply.output_format.as_deref(), Some("jpeg"));
    assert_eq!(reply.usage.input_tokens, 4);
    assert_eq!(reply.usage.output_tokens, 8);
    assert_eq!(reply.usage.total_tokens, 12);
    assert_eq!(reply.usage.prompt_tokens, Some(4));
    assert_eq!(reply.usage.completion_tokens, Some(8));
    assert_eq!(reply.usage.input_tokens_details.image_tokens, 1);
}

fn gpt_option_values() {
    let empty = r#"{"created":7,"data":[],"size":"","quality":"","output_format":""}"#;
    let mut computed = decoded(empty);
    let requested = options(r#"{"width":512,"height":256,"quality":"low"}"#);
    apply_family_defaults(ImageFamily::GptImage, &mut computed, &requested).unwrap();
    assert_eq!(computed.size.as_deref(), Some("512x256"));
    assert_eq!(computed.quality.as_deref(), Some("low"));
    assert_eq!(computed.output_format.as_deref(), Some("png"));
    let mut defaults = decoded(r#"{"created":7,"data":[]}"#);
    apply_family_defaults(ImageFamily::GptImage, &mut defaults, &options("{}")).unwrap();
    assert_eq!(defaults.size.as_deref(), Some("1024x1024"));
    assert_eq!(defaults.quality.as_deref(), Some("high"));
    assert_eq!(defaults.output_format.as_deref(), Some("png"));
}

fn b64_json_and_url() {
    let body = r#"{"created":7,"data":[{"b64_json":"YQ=="},{"url":"https://example.test/b.png","revised_prompt":"a red cat"}]}"#;
    let reply = decoded(body);
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.data[0].b64_json.as_deref(), Some("YQ=="));
    assert_eq!(reply.data[0].url, None);
    assert_eq!(
        reply.data[1].url.as_deref(),
        Some("https://example.test/b.png")
    );
    assert_eq!(reply.data[1].revised_prompt.as_deref(), Some("a red cat"));
    assert_eq!(reply.data[1].b64_json, None);
}

fn missing_created() {
    assert_eq!(decoded(r#"{"data":[]}"#).created, NOW);
    assert_eq!(decoded(r#"{"created":null,"data":[]}"#).created, NOW);
    assert_eq!(decoded(r#"{"created":123,"data":[]}"#).created, 123);
    // an explicit zero is a value the caller sent, so it stays
    assert_eq!(decoded(r#"{"created":0,"data":[]}"#).created, 0);
}

async fn image_status_error() {
    sync_status_error().await;
    async_status_error().await;
}

async fn sync_status_error() {
    let status = FakeTransport::answering(429, &[], r#"{"error":{"message":"slow down"}}"#);
    let mut runner = Runner::new();
    let status_failure = runner
        .run(call("dall-e-2", options("{}")), &status)
        .await
        .unwrap_err();
    assert!(status_failure.mapped());
    assert_eq!(status_failure.status(), Some(429));
    let transport = FakeTransport::failing();
    let transport_failure = runner
        .run(call("dall-e-2", options("{}")), &transport)
        .await
        .unwrap_err();
    assert!(transport_failure.mapped());
    assert_eq!(transport_failure.status(), Some(500));
}

async fn async_status_error() {
    let status = FakeTransport::answering(429, &[], r#"{"error":{"message":"slow down"}}"#);
    let transport = FakeTransport::failing();
    let mut runner = Runner::new();
    let mut async_call = call("dall-e-2", options("{}"));
    async_call.delegate_async = true;
    let status_failure = runner.run(async_call, &status).await.unwrap_err();
    assert!(!status_failure.mapped());
    assert_eq!(status_failure.status(), Some(429));
    let mut second_call = call("dall-e-2", options("{}"));
    second_call.delegate_async = true;
    let transport_failure = runner.run(second_call, &transport).await.unwrap_err();
    assert!(!transport_failure.mapped());
    assert_eq!(transport_failure.status(), None);
    assert!(matches!(
        transport_failure.failure_source(),
        ImageGenerationSource::Transport(_)
    ));
}

fn unsupported_image_param() {
    mapped_supported_param();
    mapped_unsupported_param();
}

fn mapped_supported_param() {
    let mut existing = IndexMap::new();
    let requested = options(r#"{"n":2}"#);
    map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &requested,
        &mut existing,
        false,
    )
    .unwrap();
    assert_eq!(existing.get("n"), requested.get("n"));
    let mut preset = options(r#"{"style":"vivid"}"#);
    let requested_style = options(r#"{"style":"natural"}"#);
    map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &requested_style,
        &mut preset,
        false,
    )
    .unwrap();
    assert_eq!(
        preset.get("style"),
        Some(&Json::Text(String::from("vivid")))
    );
}

fn mapped_unsupported_param() {
    let mut dropped = IndexMap::new();
    map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &options(r#"{"style":"vivid"}"#),
        &mut dropped,
        true,
    )
    .unwrap();
    assert!(dropped.is_empty());
    let mut empty = IndexMap::new();
    let error = map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &options(r#"{"style":"vivid"}"#),
        &mut empty,
        false,
    )
    .unwrap_err();
    let message = error.to_string();
    let ImageParamError::Unsupported {
        parameter,
        model,
        supported,
    } = error;
    assert_eq!(parameter, "style");
    assert_eq!(model, "dall-e-2");
    assert_eq!(supported, &DALL_E_2_PARAMS);
    assert!(message.contains("style") && message.contains("dall-e-2"));
    assert!(message.contains("supported parameters"));
    let mut allowed = IndexMap::new();
    map_openai_params(
        ImageFamily::DallE3,
        "dall-e-3",
        &options(r#"{"style":"vivid"}"#),
        &mut allowed,
        false,
    )
    .unwrap();
    assert!(allowed.contains_key("style"));
}

fn image_request_overlay() {
    overlay_body_and_retries();
    overlay_headers();
}

fn overlay_body_and_retries() {
    let requested = options(r#"{"model":"dall-e-3","prompt":"over","n":2,"max_retries":5}"#);
    let prepared = prepare_image_call(call("dall-e-2", requested)).unwrap();
    assert_eq!(
        prepared.data.get("model"),
        Some(&Json::Text(String::from("dall-e-3")))
    );
    assert_eq!(
        prepared.data.get("prompt"),
        Some(&Json::Text(String::from("over")))
    );
    assert_eq!(
        prepared.data.get("n"),
        Some(&Json::Number(serde_json::Number::from(2_i64)))
    );
    assert!(!prepared.data.contains_key("max_retries"));
    assert_eq!(prepared.max_retries, 5);
    let defaulted = prepare_image_call(call("dall-e-2", options("{}"))).unwrap();
    assert_eq!(defaulted.max_retries, 2);
    let failure =
        prepare_image_call(call("dall-e-2", options(r#"{"max_retries":"3"}"#))).unwrap_err();
    assert!(matches!(failure.source.status(), ErrorStatus::Code(422)));
    assert!(!failure.data.contains_key("max_retries"));
}

fn overlay_headers() {
    let requested = options(r#"{"extra_headers":{"x-old":"no"}}"#);
    let mut explicit = call("dall-e-2", requested.clone());
    explicit.headers = Some(Headers::from([(
        String::from("x-test"),
        String::from("yes"),
    )]));
    let prepared = prepare_image_call(explicit).unwrap();
    let overridden = outbound_request(&prepared).unwrap();
    assert_eq!(header_text(&overridden.headers, "x-test"), Some("yes"));
    assert_eq!(header_text(&overridden.headers, "x-old"), None);
    assert_eq!(
        header_text(&overridden.headers, "openai-organization"),
        Some("org-test")
    );
    assert!(!body_of(&overridden).contains_key("extra_headers"));
    assert_eq!(
        prepared.data.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            String::from("x-old"),
            Json::Text(String::from("no"))
        )])))
    );
    let mut empty = call("dall-e-2", requested);
    empty.headers = Some(Headers::new());
    let kept = outbound_request(&prepare_image_call(empty).unwrap()).unwrap();
    assert_eq!(header_text(&kept.headers, "x-old"), Some("no"));
    assert!(!body_of(&kept).contains_key("extra_headers"));
}

pub(super) fn header_text<'map>(headers: &'map HeaderMap, name: &str) -> Option<&'map str> {
    let value = headers.get(name)?;
    value.to_str().ok()
}

pub(super) fn body_of(request: &OutboundRequest) -> IndexMap<String, Json> {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the request body is JSON");
    };
    serde_json::from_slice(bytes).unwrap()
}

async fn image_raw_response_headers() {
    let transport = FakeTransport::answering(
        200,
        &[("x-request-id", "req-1"), ("openai-processing-ms", "42")],
        r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#,
    );
    let mut runner = Runner::new();
    raw_headers_sync(&mut runner, &transport).await;
    raw_headers_async(&mut runner, &transport).await;
}

async fn raw_headers_sync(runner: &mut Runner, transport: &FakeTransport) {
    let outcome = runner
        .run(call("dall-e-2", options("{}")), transport)
        .await
        .unwrap();
    assert_eq!(
        outcome.provider_headers.headers.get("x-request-id"),
        Some(&String::from("req-1"))
    );
    assert_eq!(
        outcome
            .provider_headers
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert_eq!(outcome.reply.created, 7);
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_sent_request(&requests[0]);
}

fn assert_sent_request(sent: &Recorded) {
    assert_eq!(sent.deadline, Duration::from_secs(30));
    assert_eq!(
        header_text(&sent.headers, AUTHORIZATION.as_str()),
        Some("Bearer sk-test")
    );
    assert_eq!(
        header_text(&sent.headers, "openai-organization"),
        Some("org-test")
    );
    assert_eq!(sent.url, "https://example.test/v1/images/generations");
    assert!(sent.query.is_empty());
    let Json::Object(request_body) = &sent.body else {
        panic!("the request body is an object");
    };
    assert_eq!(
        request_body.get("model"),
        Some(&Json::Text(String::from("dall-e-2")))
    );
}

async fn raw_headers_async(runner: &mut Runner, transport: &FakeTransport) {
    let mut async_call = call("dall-e-2", options("{}"));
    async_call.delegate_async = true;
    let outcome = runner.run(async_call, transport).await.unwrap();
    assert_eq!(
        outcome.provider_headers.headers.get("openai-processing-ms"),
        Some(&String::from("42"))
    );
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn image_generation_logs_the_pre_call_then_the_post_call() {
    let (redactor, mut state, sink) = logging();
    let snapshots = Mutex::new(Vec::new());
    let transport = ok_transport();
    let prepared = {
        let logger = |details: &Json| {
            snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(details.clone());
            Ok(())
        };
        let env = logging_env(&redactor, &sink, Some(&logger));
        let mut hooks = PrePostLogger {
            state: &mut state,
            env: &env,
        };
        begin_image_generation(call("dall-e-2", options("{}")), &mut hooks).unwrap()
    };
    let sent = send_image_generation(&prepared, &transport).await;
    {
        let logger = |details: &Json| {
            snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(details.clone());
            Ok(())
        };
        let env = logging_env(&redactor, &sink, Some(&logger));
        let mut hooks = PrePostLogger {
            state: &mut state,
            env: &env,
        };
        finish_image_generation(&prepared, sent, &mut hooks).unwrap()
    };
    assert_pre_and_post_call_records(snapshots);
}

pub(super) fn ok_transport() -> FakeTransport {
    FakeTransport::answering(
        200,
        &[],
        r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#,
    )
}

fn assert_pre_call_headers(pre: &IndexMap<String, Json>) {
    let Some(Json::Object(arguments)) = pre.get("additional_args") else {
        panic!("the pre-call record carries additional_args");
    };
    let Some(Json::Object(sent)) = arguments.get("headers") else {
        panic!("the pre-call record carries the request headers");
    };
    assert!(sent.contains_key("Authorization"));
}

fn assert_pre_and_post_call_records(snapshots: Mutex<Vec<Json>>) {
    let recorded = snapshots
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 2);
    let phases: Vec<_> = recorded
        .iter()
        .map(|details| match details {
            Json::Object(fields) => fields.get("log_event_type").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            Some(Json::Text(String::from("pre_api_call"))),
            Some(Json::Text(String::from("post_api_call")))
        ]
    );
    let Json::Object(pre) = &recorded[0] else {
        panic!("the pre-call record is an object");
    };
    assert_eq!(pre.get("input"), Some(&Json::Text(String::from("a cat"))));
    assert_pre_call_headers(pre);
    let Json::Object(post) = &recorded[1] else {
        panic!("the post-call record is an object");
    };
    assert_eq!(post.get("input"), Some(&Json::Text(String::from("a cat"))));
    let Some(Json::Object(arguments)) = post.get("additional_args") else {
        panic!("the post-call record carries additional_args");
    };
    let Some(Json::Object(body)) = arguments.get("complete_input_dict") else {
        panic!("the post-call record carries the request body");
    };
    assert_eq!(
        body.get("model"),
        Some(&Json::Text(String::from("dall-e-2")))
    );
}

#[test]
fn replays_the_openai_image_generation_corpus() {
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_image_generation");
    let mut paths = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no image generation corpus seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_image_reply(&bytes, NOW);
            if name.starts_with("valid-") {
                assert!(decoded.is_ok(), "{name}: {decoded:?}");
            } else {
                assert!(decoded.is_err(), "{name}: {decoded:?}");
            }
            Ok::<(), IoError>(())
        })
        .unwrap();
}

#[test]
fn image_generation_failure_is_mapped_only_for_the_sync_facade() {
    let wrapped = ImageGenerationFailure::Wrapped(OpenAiImageError::wrapped(
        ImageGenerationSource::MaxRetries,
    ));
    assert!(wrapped.mapped());
    assert_eq!(wrapped.status(), Some(422));
    let raw = ImageGenerationFailure::Raw(ImageGenerationSource::MaxRetries);
    assert!(!raw.mapped());
    assert_eq!(raw.status(), Some(422));
}

#[rstest]
#[case::message_defaults_to_422(r#"{"error":{"message":"boom"}}"#, Some((422, "boom")))]
#[case::integer_code_is_the_status(r#"{"error":{"message":"slow","code":429}}"#, Some((429, "slow")))]
#[case::code_alone_gets_the_default_text(r#"{"error":{"code":429}}"#, Some((429, "Error in response object")))]
#[case::text_code_falls_back_to_422(r#"{"error":{"message":"x","code":"bad"}}"#, Some((422, "x")))]
#[case::oversized_code_falls_back_to_422(r#"{"error":{"message":"x","code":70000}}"#, Some((422, "x")))]
#[case::negative_code_falls_back_to_422(r#"{"error":{"message":"x","code":-1}}"#, Some((422, "x")))]
#[case::empty_message_with_a_code(r#"{"error":{"message":"","code":429}}"#, Some((429, "")))]
#[case::empty_object_is_no_error(r#"{"error":{}}"#, None)]
#[case::empty_message_and_null_code(r#"{"error":{"message":"","code":null}}"#, None)]
#[case::null_error(r#"{"error":null}"#, None)]
#[case::empty_text_error(r#"{"error":""}"#, None)]
#[case::text_error(r#"{"error":"boom"}"#, Some((422, "Error in response object")))]
#[case::true_error(r#"{"error":true}"#, Some((422, "Error in response object")))]
#[case::number_error(r#"{"error":5}"#, Some((422, "Error in response object")))]
#[case::list_error(r#"{"error":[1]}"#, Some((422, "Error in response object")))]
#[case::number_message(r#"{"error":{"message":5}}"#, Some((422, "5")))]
#[case::fraction_message(r#"{"error":{"message":1.5}}"#, Some((422, "1.5")))]
#[case::zero_message(r#"{"error":{"message":0}}"#, None)]
#[case::true_message(r#"{"error":{"message":true}}"#, Some((422, "true")))]
#[case::false_message(r#"{"error":{"message":false}}"#, None)]
#[case::empty_list_message(r#"{"error":{"message":[]}}"#, None)]
#[case::list_message(r#"{"error":{"message":[1]}}"#, Some((422, "[1]")))]
#[case::empty_object_message(r#"{"error":{"message":{}}}"#, None)]
#[case::object_message(r#"{"error":{"message":{"a":1}}}"#, Some((422, r#"{"a":1}"#)))]
fn response_object_errors(#[case] body: &str, #[case] expected: Option<(u16, &str)>) {
    match (decode_image_reply(body.as_bytes(), NOW), expected) {
        (Ok(_), None) => {}
        (Err(ImageReplyError::ResponseObject { status, message }), Some((code, text))) => {
            assert_eq!(status, code, "{body}");
            assert_eq!(message, text, "{body}");
        }
        (found, wanted) => panic!("{body}: {found:?} against {wanted:?}"),
    }
}

#[rstest]
#[case::error_message(r#"{"error":{"message":"slow down"}}"#, "slow down")]
#[case::plain_text("service unavailable", "service unavailable")]
#[case::numeric_message(r#"{"error":{"message":5}}"#, r#"{"error":{"message":5}}"#)]
#[case::text_error(r#"{"error":"nope"}"#, r#"{"error":"nope"}"#)]
#[case::list_body("[1]", "[1]")]
#[tokio::test]
async fn status_error_messages(#[case] body: &str, #[case] expected: &str) {
    let transport = FakeTransport::answering(503, &[], body);
    let failure = Runner::new()
        .run(call("dall-e-2", options("{}")), &transport)
        .await
        .unwrap_err();
    assert_eq!(failure.failure_source().to_string(), expected);
    assert_eq!(failure.status(), Some(503));
}

async fn serve_image_reply(listener: TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !received.ends_with(b"}") {
        let read = stream.read(&mut chunk).await.unwrap();
        received.extend_from_slice(&chunk[..read]);
    }
    let body = r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#;
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-request-id: req-9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(received).unwrap().to_lowercase()
}

#[tokio::test]
async fn proxied_client_posts_to_the_images_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut local = call("dall-e-2", options(r#"{"n":2}"#));
    local.api_base = Some(format!("http://{address}/v1/"));
    let prepared = prepare_image_call(local).unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let (head, sent) = tokio::join!(
        serve_image_reply(listener),
        send_image_generation(&prepared, &client)
    );
    let outcome = sent.unwrap();
    assert!(
        head.starts_with("post /v1/images/generations http/1.1"),
        "{head}"
    );
    assert!(head.contains("authorization: bearer sk-test"), "{head}");
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains(r#""model":"dall-e-2""#), "{head}");
    assert_eq!(outcome.reply.created, 7);
    assert_eq!(
        outcome.provider_headers.headers.get("x-request-id"),
        Some(&String::from("req-9"))
    );
}

#[test]
fn transform_image_reply_logs_the_reply_then_applies_the_family_defaults() {
    let (redactor, mut state, sink) = logging();
    let snapshots = Mutex::new(Vec::new());
    let params = options("{}");
    let request_data = options(r#"{"model":"dall-e-3","prompt":"a cat"}"#);
    let call = ImageTransformCall {
        family: ImageFamily::DallE3,
        raw: br#"{"data":[{"url":"https://example.test/a.png"}]}"#,
        optional_params: &params,
        request_data: &request_data,
        created_seconds: NOW,
    };
    let reply = {
        let logger = |details: &Json| {
            snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(details.clone());
            Ok(())
        };
        let env = logging_env(&redactor, &sink, Some(&logger));
        transform_image_reply(call, &mut state, &env).unwrap()
    };
    assert_eq!(reply.created, NOW);
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.quality.as_deref(), Some("hd"));
    let recorded = snapshots
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 1);
    let Json::Object(post) = &recorded[0] else {
        panic!("the post-call record is an object");
    };
    assert_eq!(
        post.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    assert_eq!(post.get("input"), Some(&Json::Text(String::from("a cat"))));
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
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

use super::call::{ImageGenerationCall, prepare_image_call};
use super::decode::decode_image_reply;
use super::defaults::apply_family_defaults;
use super::error::{ErrorStatus, ImageGenerationFailure, ImageGenerationSource};
use super::family::{
    DALL_E_2_PARAMS, DALL_E_3_PARAMS, GPT_IMAGE_PARAMS, ImageFamily, ImageParamError,
    map_openai_params, select_image_family,
};
use super::logging::PrePostLogger;
use super::reply::ImageGenerationReply;
use super::request::{
    ImageGenerationOutcome, begin_image_generation, finish_image_generation, send_image_generation,
};
use super::transport::ImageGenerationTransport;
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

    async fn run<T: ImageGenerationTransport>(
        &mut self,
        call: ImageGenerationCall,
        transport: &T,
    ) -> Result<ImageGenerationOutcome, ImageGenerationFailure> {
        let env = logging_env(&self.redactor, &self.sink, None);
        let prepared = {
            let mut logger = PrePostLogger {
                state: &mut self.state,
                env: &env,
            };
            begin_image_generation(call, &mut logger)?
        };
        let sent = send_image_generation(&prepared, transport).await;
        let mut logger = PrePostLogger {
            state: &mut self.state,
            env: &env,
        };
        finish_image_generation(&prepared, sent, &mut logger)
    }
}

fn options(text: &str) -> IndexMap<String, Json> {
    let parsed = serde_json::from_str(text).unwrap();
    parsed
}

fn decoded(body: &str) -> ImageGenerationReply {
    let bytes = body.as_bytes();
    decode_image_reply(bytes, NOW).unwrap()
}

fn call(model: &str, optional_params: IndexMap<String, Json>) -> ImageGenerationCall {
    ImageGenerationCall {
        model: model.to_owned(),
        prompt: String::from("a cat"),
        optional_params,
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some(API_BASE.to_owned()),
        organization: Some(String::from("org-test")),
        headers: None,
        timeout: Some(Duration::from_secs(30)),
        delegate_async: false,
        now_seconds: NOW,
    }
}

#[derive(Clone)]
struct Recorded {
    body: Json,
    deadline: Duration,
    headers: HeaderMap,
}

struct FakeTransport {
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

    fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageGenerationTransport for FakeTransport {
    async fn generate(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
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
                headers: request.headers.clone(),
            });
        if self.failure {
            return Err(TransportError::Closed);
        }
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
    let mut reply = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut reply, &options("{}")).unwrap();
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.quality.as_deref(), Some("standard"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));
    let supplied = options(r#"{"size":"512x512","quality":"low"}"#);
    let mut reply = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut reply, &supplied).unwrap();
    assert_eq!(reply.size.as_deref(), Some("512x512"));
    assert_eq!(reply.quality.as_deref(), Some("low"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));
    let explicit_null = options(r#"{"quality":null}"#);
    let mut reply = decoded(body);
    apply_family_defaults(ImageFamily::DallE2, &mut reply, &explicit_null).unwrap();
    assert_eq!(reply.quality, None);
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
    let mut reply = decoded(body);
    apply_family_defaults(ImageFamily::DallE3, &mut reply, &options("{}")).unwrap();
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.quality.as_deref(), Some("hd"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));
}

fn gpt_image_usage() {
    assert_eq!(select_image_family("gpt-image-1"), ImageFamily::GptImage);
    // the selector is case-sensitive, so an upper-case DALL-E name is a GPT image
    assert_eq!(select_image_family("DALL-E-3"), ImageFamily::GptImage);
    assert_eq!(ImageFamily::GptImage.supported_params(), &GPT_IMAGE_PARAMS);
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
    let empty = r#"{"created":7,"data":[],"size":"","quality":"","output_format":""}"#;
    let mut reply = decoded(empty);
    let requested = options(r#"{"width":512,"height":256,"quality":"low"}"#);
    apply_family_defaults(ImageFamily::GptImage, &mut reply, &requested).unwrap();
    assert_eq!(reply.size.as_deref(), Some("512x256"));
    assert_eq!(reply.quality.as_deref(), Some("low"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));
    let mut reply = decoded(r#"{"created":7,"data":[]}"#);
    apply_family_defaults(ImageFamily::GptImage, &mut reply, &options("{}")).unwrap();
    assert_eq!(reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(reply.quality.as_deref(), Some("high"));
    assert_eq!(reply.output_format.as_deref(), Some("png"));
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
    let status = FakeTransport::answering(429, &[], r#"{"error":{"message":"slow down"}}"#);
    let mut sync_runner = Runner::new();
    let failure = sync_runner
        .run(call("dall-e-2", options("{}")), &status)
        .await
        .unwrap_err();
    assert!(failure.mapped());
    assert_eq!(failure.status(), Some(429));
    let transport = FakeTransport::failing();
    let failure = sync_runner
        .run(call("dall-e-2", options("{}")), &transport)
        .await
        .unwrap_err();
    assert!(failure.mapped());
    assert_eq!(failure.status(), Some(500));
    let mut async_runner = Runner::new();
    let mut async_call = call("dall-e-2", options("{}"));
    async_call.delegate_async = true;
    let failure = async_runner.run(async_call, &status).await.unwrap_err();
    assert!(!failure.mapped());
    assert_eq!(failure.status(), Some(429));
    let mut async_call = call("dall-e-2", options("{}"));
    async_call.delegate_async = true;
    let failure = async_runner.run(async_call, &transport).await.unwrap_err();
    assert!(!failure.mapped());
    assert!(matches!(failure.status(), None));
    assert!(matches!(
        failure.source(),
        ImageGenerationSource::Transport(_)
    ));
}

fn unsupported_image_param() {
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
    let mut existing = options(r#"{"style":"vivid"}"#);
    let requested = options(r#"{"style":"natural"}"#);
    map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &requested,
        &mut existing,
        false,
    )
    .unwrap();
    assert_eq!(
        existing.get("style"),
        Some(&Json::Text(String::from("vivid")))
    );
    let mut existing = IndexMap::new();
    map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &options(r#"{"style":"vivid"}"#),
        &mut existing,
        true,
    )
    .unwrap();
    assert!(existing.is_empty());
    let mut existing = IndexMap::new();
    let error = map_openai_params(
        ImageFamily::DallE2,
        "dall-e-2",
        &options(r#"{"style":"vivid"}"#),
        &mut existing,
        false,
    )
    .unwrap_err();
    let ImageParamError::Unsupported {
        parameter,
        model,
        supported,
    } = error;
    assert_eq!(parameter, "style");
    assert_eq!(model, "dall-e-2");
    assert_eq!(supported, &DALL_E_2_PARAMS);
    let message = ImageParamError::Unsupported {
        parameter: String::from("style"),
        model: String::from("dall-e-2"),
        supported: &DALL_E_2_PARAMS,
    }
    .to_string();
    assert!(message.contains("style") && message.contains("dall-e-2"));
    assert!(message.contains("supported parameters"));
    let mut existing = IndexMap::new();
    map_openai_params(
        ImageFamily::DallE3,
        "dall-e-3",
        &options(r#"{"style":"vivid"}"#),
        &mut existing,
        false,
    )
    .unwrap();
    assert!(existing.contains_key("style"));
}

fn image_request_overlay() {
    let requested = options(
        r#"{"model":"dall-e-3","prompt":"over","n":2,"max_retries":5,"extra_headers":{"x-old":"no"}}"#,
    );
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
    assert_eq!(
        prepared.data.get("extra_headers"),
        prepared.wire_data.get("extra_headers")
    );
    assert_eq!(
        prepare_image_call(call("dall-e-2", options("{}")))
            .unwrap()
            .max_retries,
        2
    );
    let failure =
        prepare_image_call(call("dall-e-2", options(r#"{"max_retries":"3"}"#))).unwrap_err();
    assert!(matches!(failure.source.status(), ErrorStatus::Code(422)));
    assert!(!failure.data.contains_key("max_retries"));
    let mut explicit = call("dall-e-2", options(r#"{"extra_headers":{"x-old":"no"}}"#));
    explicit.headers = Some(Headers::from([(
        String::from("x-test"),
        String::from("yes"),
    )]));
    let prepared = prepare_image_call(explicit).unwrap();
    assert_eq!(
        prepared.wire_data.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            String::from("x-test"),
            Json::Text(String::from("yes"))
        )])))
    );
    assert_eq!(
        prepared.data.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            String::from("x-old"),
            Json::Text(String::from("no"))
        )])))
    );
    let mut empty = call("dall-e-2", options(r#"{"extra_headers":{"x-old":"no"}}"#));
    empty.headers = Some(Headers::new());
    let prepared = prepare_image_call(empty).unwrap();
    assert_eq!(
        prepared.wire_data.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            String::from("x-old"),
            Json::Text(String::from("no"))
        )])))
    );
}

async fn image_raw_response_headers() {
    let transport = FakeTransport::answering(
        200,
        &[("x-request-id", "req-1"), ("openai-processing-ms", "42")],
        r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#,
    );
    let mut runner = Runner::new();
    let outcome = runner
        .run(call("dall-e-2", options("{}")), &transport)
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
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].deadline, Duration::from_secs(30));
    assert_eq!(
        requests[0]
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-test")
    );
    assert_eq!(
        requests[0].body.get("model"),
        Some(&Json::Text(String::from("dall-e-2")))
    );
    let mut async_call = call("dall-e-2", options("{}"));
    async_call.delegate_async = true;
    let outcome = runner.run(async_call, &transport).await.unwrap();
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
    let logger = |details: &Json| {
        snapshots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = logging_env(&redactor, &sink, Some(&logger));
    let transport = FakeTransport::answering(
        200,
        &[],
        r#"{"created":7,"data":[{"url":"https://example.test/a.png"}]}"#,
    );
    let prepared = {
        let mut hooks = PrePostLogger {
            state: &mut state,
            env: &env,
        };
        begin_image_generation(call("dall-e-2", options("{}")), &mut hooks).unwrap()
    };
    let sent = send_image_generation(&prepared, &transport).await;
    let mut hooks = PrePostLogger {
        state: &mut state,
        env: &env,
    };
    finish_image_generation(&prepared, sent, &mut hooks).unwrap();
    let recorded = snapshots
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 2);
    let phases: Vec<_> = recorded
        .iter()
        .map(|details| match details {
            Json::Object(fields) => fields.get("log_event_type").cloned(),
            _ => None,
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
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Text(String::from("a cat")))
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
            Ok::<(), std::io::Error>(())
        })
        .unwrap();
}

#[test]
fn image_generation_failure_is_mapped_only_for_the_sync_facade() {
    let wrapped = ImageGenerationFailure::Wrapped(super::error::OpenAiImageError::wrapped(
        ImageGenerationSource::MaxRetries,
    ));
    assert!(wrapped.mapped());
    assert_eq!(wrapped.status(), Some(422));
    let raw = ImageGenerationFailure::Raw(ImageGenerationSource::MaxRetries);
    assert!(!raw.mapped());
    assert_eq!(raw.status(), Some(422));
}

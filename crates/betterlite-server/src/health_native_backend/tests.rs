#![cfg(test)]

use std::error::Error as StdError;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::{DecisionsJson, Json};
use betterlite_ocr::json::{Json as OcrJson, Object as OcrObject};
use betterlite_ocr::ocr_error::OcrError;
use betterlite_sdk::call_wrapper::error::CallError;
use rstest::rstest;
use secrecy::SecretString;

use self::support::{
    HEALTHY_HEADERS, Harness, MockServer, OwnerCall, document, headers_map, json, mode_parameters,
    ocr_client, params, probe_call, probe_logging, text,
};
use super::ocr::{OcrProbe, aocr_request};
use super::seams::OcrProbeCall as _;
use crate::health_check_entry::HealthCheckOutcome;
use crate::health_probe_modes::backend::{CheapestChatModels as _, HealthProbeBackend as _};
use crate::health_probe_modes::calls::{
    AudioTranscriptionCall, ImageEditCall, OcrCall, RealtimeCall,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::evaluation::evaluation_call;
use crate::health_probe_modes::logging::HealthProbeLogging;
use crate::health_probe_modes::response::{HealthCheckResponse, health_check_response};

pub(super) mod support;

const OCR_ANSWER: &[u8] =
    br##"{"pages":[{"index":0,"markdown":"# Title"}],"usage_info":{"pages_processed":1}}"##;
const OCR_REJECTION: &[u8] = br#"{"message":"invalid api key"}"#;

fn healthy(outcome: HealthCheckOutcome) -> HealthCheckResponse {
    match outcome {
        HealthCheckOutcome::Healthy(response) => response,
        HealthCheckOutcome::Failed(failure) => panic!("the health check fails: {}", failure.error),
    }
}

fn ocr_json(value: &str) -> OcrJson {
    serde_json::from_str(value).expect("the test OCR JSON parses")
}

fn ocr_body(request: &str) -> OcrObject {
    let body = request
        .split_once("\r\n\r\n")
        .expect("the request has a body")
        .1;
    serde_json::from_str(body).expect("the request body is JSON")
}

#[tokio::test]
async fn backend_chat_sends_the_call_through_the_sdk_target() {
    let harness = Harness::new();
    harness.chat.answer(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[("model", text("gpt-4o")), ("messages", json("[]"))]);
    let reply = harness
        .backend()
        .chat(probe_call(parameters.clone()))
        .await
        .expect("the chat call succeeds");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.chat.take();
    let mut expected = parameters;
    expected.insert(String::from("api_key"), text("sk-test"));
    assert_eq!(call.request, expected);
}

#[tokio::test]
async fn backend_chat_without_an_api_key_sends_no_key() {
    let harness = Harness::new();
    let parameters = mode_parameters(&[("model", text("gpt-4o"))]);
    let mut call = probe_call(parameters.clone());
    call.api_key = None;
    let _reply = harness
        .backend()
        .chat(call)
        .await
        .expect("the chat call succeeds");
    assert_eq!(harness.chat.take().request, parameters);
}

#[tokio::test]
async fn backend_chat_carries_no_headers_from_a_stream() {
    let harness = Harness::new();
    harness.chat.stream();
    let reply = harness
        .backend()
        .chat(probe_call(mode_parameters(&[("model", text("gpt-4o"))])))
        .await
        .expect("the stream call succeeds");
    assert!(reply.hidden_headers.is_empty());
}

#[tokio::test]
async fn backend_chat_hands_the_logging_state_to_the_call() {
    let harness = Harness::new();
    let logging = probe_logging();
    let state = logging
        .logging_state()
        .expect("the logging object exposes its state");
    let handle: Arc<dyn HealthProbeLogging> = Arc::new(logging);
    let mut call = probe_call(mode_parameters(&[("model", text("gpt-4o"))]));
    call.logging = Some(handle);
    let _reply = harness
        .backend()
        .chat(call)
        .await
        .expect("the chat call succeeds");
    let recorded = harness
        .chat
        .take()
        .logging
        .expect("the call carries a logging handle");
    assert!(Arc::ptr_eq(&recorded.state, &state));
}

#[tokio::test]
async fn backend_completion_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("gpt-3.5-turbo-instruct")),
        ("prompt", text("test")),
    ]);
    let handle: Arc<dyn HealthProbeLogging> = Arc::new(probe_logging());
    let mut probe = probe_call(parameters.clone());
    probe.logging = Some(handle);
    let reply = harness
        .backend()
        .completion(probe)
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "completion");
    assert_eq!(call.parameters(), &parameters);
    let OwnerCall::Mode {
        api_key, logging, ..
    } = call
    else {
        panic!("the owner records a completion call");
    };
    assert_eq!(api_key.as_deref(), Some("sk-test"));
    assert!(logging);
}

#[tokio::test]
async fn backend_embedding_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("text-embedding-3-small")),
        ("input", json(r#"["test"]"#)),
    ]);
    let reply = harness
        .backend()
        .embedding(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "embedding");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_audio_speech_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("tts-1")),
        ("input", text("test")),
        ("voice", text("alloy")),
    ]);
    let reply = harness
        .backend()
        .audio_speech(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "audio_speech");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_audio_transcription_forwards_the_file() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[("model", text("whisper-1")), ("language", text("en"))]);
    let call = AudioTranscriptionCall {
        parameters: parameters.clone(),
        file: b"audio".to_vec(),
        api_key: Some(SecretString::from("sk-audio")),
        logging: None,
    };
    let reply = harness
        .backend()
        .audio_transcription(call)
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let recorded = harness.owner.take();
    assert_eq!(recorded.mode_name(), "audio_transcription");
    assert_eq!(recorded.parameters(), &parameters);
    let OwnerCall::Transcription {
        file,
        api_key,
        logging,
        ..
    } = recorded
    else {
        panic!("the owner records a transcription call");
    };
    assert_eq!(file, b"audio".to_vec());
    assert_eq!(api_key.as_deref(), Some("sk-audio"));
    assert!(!logging);
}

#[tokio::test]
async fn backend_image_generation_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters =
        mode_parameters(&[("model", text("dall-e-3")), ("prompt", text("a red cube"))]);
    let reply = harness
        .backend()
        .image_generation(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "image_generation");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_image_edit_forwards_the_image() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters =
        mode_parameters(&[("model", text("gpt-image-1")), ("size", text("1024x1024"))]);
    let call = ImageEditCall {
        parameters: parameters.clone(),
        image: b"png".to_vec(),
        prompt: "a health check image",
        api_key: Some(SecretString::from("sk-image")),
        logging: None,
    };
    let reply = harness
        .backend()
        .image_edit(call)
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let recorded = harness.owner.take();
    assert_eq!(recorded.mode_name(), "image_edit");
    assert_eq!(recorded.parameters(), &parameters);
    let OwnerCall::ImageEdit {
        image,
        prompt,
        api_key,
        logging,
        ..
    } = recorded
    else {
        panic!("the owner records an image edit call");
    };
    assert_eq!(image, b"png".to_vec());
    assert_eq!(prompt, "a health check image");
    assert_eq!(api_key.as_deref(), Some("sk-image"));
    assert!(!logging);
}

#[tokio::test]
async fn backend_rerank_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("rerank-v3")),
        ("query", text("test")),
        ("documents", json(r#"["my sample text"]"#)),
    ]);
    let reply = harness
        .backend()
        .rerank(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "rerank");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_batch_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("gpt-4o")),
        ("custom_llm_provider", text("openai")),
        ("messages", json("[]")),
    ]);
    let reply = harness
        .backend()
        .batch(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "batch");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_responses_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[("model", text("gpt-4o")), ("input", text("test"))]);
    let reply = harness
        .backend()
        .responses(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "responses");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_ocr_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("mistral-ocr-latest")),
        ("custom_llm_provider", text("mistral")),
    ]);
    let source = "data:application/pdf;base64,aGVhbHRo";
    let call = OcrCall {
        parameters: parameters.clone(),
        document: document(source),
        api_key: Some(SecretString::from("sk-ocr")),
        logging: None,
    };
    let reply = harness
        .backend()
        .ocr(call)
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let recorded = harness.owner.take();
    assert_eq!(recorded.mode_name(), "ocr");
    assert_eq!(recorded.parameters(), &parameters);
    let OwnerCall::Ocr {
        document: seen,
        api_key,
        logging,
        ..
    } = recorded
    else {
        panic!("the owner records an ocr call");
    };
    assert_eq!(seen, document(source));
    assert_eq!(api_key.as_deref(), Some("sk-ocr"));
    assert!(!logging);
}

#[tokio::test]
async fn backend_video_generation_forwards_the_call() {
    let harness = Harness::new();
    harness.owner.reply(&HEALTHY_HEADERS);
    let parameters = mode_parameters(&[
        ("model", text("sora-2")),
        ("custom_llm_provider", text("openai")),
        ("prompt", text("test video generation")),
    ]);
    let reply = harness
        .backend()
        .video_generation(probe_call(parameters.clone()))
        .await
        .expect("the owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let call = harness.owner.take();
    assert_eq!(call.mode_name(), "video_generation");
    assert_eq!(call.parameters(), &parameters);
}

#[tokio::test]
async fn backend_realtime_reports_the_mode_as_not_built() {
    let harness = Harness::new();
    let call = RealtimeCall {
        model: String::from("gpt-4o-realtime-preview"),
        custom_llm_provider: String::from("openai"),
        api_base: None,
        api_key: None,
        api_version: None,
        model_params: mode_parameters(&[("model", text("gpt-4o-realtime-preview"))]),
        logging: None,
    };
    let error = harness
        .backend()
        .realtime(call)
        .await
        .expect_err("the mode is not built");
    assert_eq!(error.to_string(), "the realtime health check is not built");
    assert_eq!(harness.owner.count(), 0);
}

#[tokio::test]
async fn backend_evaluation_forwards_the_call() {
    let harness = Harness::new();
    harness.evaluation.reply(&HEALTHY_HEADERS);
    let probe_params = params(&[
        ("model", text("typesafe/gpt-4o")),
        ("api_base", text("https://api.typesafe.ai/v1")),
        ("custom_llm_provider", text("typesafe")),
    ]);
    let call = evaluation_call(&probe_params, Some("summarize"), "typesafe")
        .expect("the evaluation call builds");
    let reply = harness
        .backend()
        .evaluation(call)
        .await
        .expect("the evaluation owner answers");
    assert_eq!(reply.hidden_headers, headers_map(&HEALTHY_HEADERS));
    let recorded = harness.evaluation.take();
    assert_eq!(recorded.params.model, "typesafe/gpt-4o");
    assert_eq!(
        recorded.params.state,
        DecisionsJson::Text(String::from("summarize"))
    );
    assert_eq!(
        recorded.params.custom_llm_provider.as_deref(),
        Some("typesafe")
    );
    assert_eq!(
        recorded.params.api_base.as_deref(),
        Some("https://api.typesafe.ai/v1")
    );
    let questions =
        serde_json::to_string(&recorded.params.questions).expect("the questions encode to JSON");
    assert!(questions.contains(r#""reachable""#), "{questions}");
    assert!(questions.contains(r#""noul""#), "{questions}");
    assert!(recorded.logging.is_none());
}

#[tokio::test]
async fn error_evaluation_keeps_the_owner_error_as_source() {
    let harness = Harness::new();
    harness.evaluation.fail("the evaluation owner is down");
    let probe_params = params(&[("model", text("typesafe/gpt-4o"))]);
    let call =
        evaluation_call(&probe_params, None, "typesafe").expect("the evaluation call builds");
    let error = harness
        .backend()
        .evaluation(call)
        .await
        .expect_err("the owner call fails");
    assert_eq!(error.to_string(), "the evaluation owner is down");
    let source = StdError::source(&error).expect("the owner error is the source");
    assert_eq!(
        source
            .downcast_ref::<io::Error>()
            .expect("the source is the owner's error")
            .to_string(),
        "the evaluation owner is down"
    );
    assert_eq!(harness.evaluation.count(), 1);
}

#[test]
fn backend_ocr_request_splits_the_connection_keywords() {
    let call = OcrCall {
        parameters: mode_parameters(&[
            ("model", text("mistral-ocr-latest")),
            ("api_base", text("https://api.test/v1")),
            ("custom_llm_provider", text("mistral")),
            ("timeout", json("30")),
            ("extra_headers", json(r#"{"x-test":"1"}"#)),
            ("pages", json("[0]")),
        ]),
        document: document("data:application/pdf;base64,YWJj"),
        api_key: None,
        logging: None,
    };
    let request = aocr_request(call).expect("the request converts");
    assert_eq!(request.model, "mistral-ocr-latest");
    assert_eq!(request.custom_llm_provider.as_deref(), Some("mistral"));
    assert_eq!(request.transport.timeout, Some(Duration::from_secs(30)));
    assert_eq!(
        request.transport.extra_headers,
        vec![(String::from("x-test"), String::from("1"))]
    );
    assert_eq!(request.optional_params.get("pages"), Some(&ocr_json("[0]")));
    assert_eq!(
        request.credentials.api_base.map(|sourced| sourced.value),
        Some(String::from("https://api.test/v1"))
    );
}

#[test]
fn backend_ocr_request_requires_a_model() {
    let call = OcrCall {
        parameters: mode_parameters(&[("custom_llm_provider", text("mistral"))]),
        document: document("data:application/pdf;base64,YWJj"),
        api_key: None,
        logging: None,
    };
    let Err(error) = aocr_request(call) else {
        panic!("the request needs a model");
    };
    let HealthProbeError::ParameterValue { key, .. } = error else {
        panic!("the conversion names the model parameter");
    };
    assert_eq!(key, "model");
}

#[tokio::test]
async fn backend_ocr_probe_passes_the_arguments_through_aocr() {
    let server = MockServer::start(200, OCR_ANSWER).await;
    let probe = OcrProbe { client: ocr_client };
    let parameters = mode_parameters(&[
        ("model", text("mistral-ocr-latest")),
        ("api_base", text(&format!("{}/v1", server.base))),
        ("custom_llm_provider", text("mistral")),
        ("include_image_base64", json("true")),
    ]);
    let source = "data:application/pdf;base64,YWJj";
    let call = OcrCall {
        parameters,
        document: document(source),
        api_key: Some(SecretString::from("sk-test")),
        logging: None,
    };
    let reply = probe.ocr(call).await.expect("the probe answers");
    assert!(reply.hidden_headers.is_empty());
    let sent = server.requests().await;
    let request = sent.first().expect("one provider call was sent");
    let lowered = request.to_lowercase();
    assert!(lowered.starts_with("post /v1/ocr"), "{lowered}");
    assert!(
        lowered.contains("authorization: bearer sk-test"),
        "{lowered}"
    );
    let body = ocr_body(request);
    assert_eq!(
        body.get("model"),
        Some(&ocr_json(r#""mistral-ocr-latest""#))
    );
    let expected_document =
        ocr_json(r#"{"type":"document_url","document_url":"data:application/pdf;base64,YWJj"}"#);
    assert_eq!(body.get("document"), Some(&expected_document));
    assert_eq!(body.get("include_image_base64"), Some(&ocr_json("true")));
}

#[tokio::test]
async fn error_chat_keeps_the_owner_error_and_does_not_retry() {
    let harness = Harness::new();
    harness.chat.fail("the provider is down");
    let error = harness
        .backend()
        .chat(probe_call(mode_parameters(&[("model", text("gpt-4o"))])))
        .await
        .expect_err("the chat call fails");
    assert_eq!(error.to_string(), "the provider is down");
    let HealthProbeError::Backend { class, .. } = &error else {
        panic!("the chat failure is a backend error");
    };
    assert!(matches!(class, ErrorClass::Authentication(_)));
    assert_eq!(class.metadata().status_code, 401);
    let source = StdError::source(&error).expect("the owner error is the source");
    assert!(source.downcast_ref::<CallError>().is_some());
    assert_eq!(harness.chat.calls(), 1);
    assert_eq!(harness.chat.retries(), 0);
}

#[tokio::test]
async fn error_completion_keeps_the_owner_error_as_source() {
    let harness = Harness::new();
    harness.owner.fail("the owner is down");
    let error = harness
        .backend()
        .completion(probe_call(mode_parameters(&[("model", text("gpt-4o"))])))
        .await
        .expect_err("the owner call fails");
    assert_eq!(error.to_string(), "the owner is down");
    let source = StdError::source(&error).expect("the owner error is the source");
    assert_eq!(
        source
            .downcast_ref::<io::Error>()
            .expect("the source is the owner's error")
            .to_string(),
        "the owner is down"
    );
}

#[tokio::test]
async fn error_ocr_probe_keeps_the_aocr_error_as_source() {
    let server = MockServer::start(401, OCR_REJECTION).await;
    let probe = OcrProbe { client: ocr_client };
    let api_base = format!("{}/v1", server.base);
    let call = OcrCall {
        parameters: mode_parameters(&[
            ("model", text("mistral-ocr-latest")),
            ("api_base", text(&api_base)),
            ("custom_llm_provider", text("mistral")),
        ]),
        document: document("data:application/pdf;base64,YWJj"),
        api_key: Some(SecretString::from("sk-test")),
        logging: None,
    };
    let error = probe.ocr(call).await.expect_err("the provider is rejected");
    let source = StdError::source(&error).expect("the aocr error is the source");
    let ocr = source
        .downcast_ref::<OcrError>()
        .expect("the source is the aocr error");
    assert_eq!(error.to_string(), ocr.to_string());
    let HealthProbeError::Backend { class, .. } = &error else {
        panic!("the ocr failure is a backend error");
    };
    assert!(matches!(class, ErrorClass::Authentication(_)));
    assert_eq!(class.metadata().status_code, 401);
    let sent = server.requests().await;
    assert_eq!(sent.len(), 1);
}

#[rstest]
#[case::authentication(401_u16)]
#[case::permission_denied(403)]
#[case::not_found(404)]
#[case::request_timeout(408)]
#[case::unprocessable(422)]
#[case::rate_limit(429)]
#[case::internal_server(500)]
#[case::not_implemented(501)]
#[case::bad_gateway(502)]
#[case::service_unavailable(503)]
#[case::gateway_timeout(504)]
#[tokio::test]
async fn error_ocr_probe_maps_each_provider_status_to_its_class(#[case] status: u16) {
    let probe = OcrProbe { client: ocr_client };
    let server = MockServer::start(status, OCR_REJECTION).await;
    let api_base = format!("{}/v1", server.base);
    let call = OcrCall {
        parameters: mode_parameters(&[
            ("model", text("mistral-ocr-latest")),
            ("api_base", text(&api_base)),
            ("custom_llm_provider", text("mistral")),
        ]),
        document: document("data:application/pdf;base64,YWJj"),
        api_key: Some(SecretString::from("sk-test")),
        logging: None,
    };
    let error = probe.ocr(call).await.expect_err("the provider is rejected");
    let HealthProbeError::Backend { class, .. } = &error else {
        panic!("the ocr failure is a backend error");
    };
    match status {
        401 => assert!(matches!(class, ErrorClass::Authentication(_))),
        403 => assert!(matches!(class, ErrorClass::PermissionDenied(_))),
        404 => assert!(matches!(class, ErrorClass::NotFound(_))),
        408 | 504 => assert!(matches!(class, ErrorClass::Timeout(_))),
        422 => assert!(matches!(class, ErrorClass::BadRequest(_))),
        429 => assert!(matches!(class, ErrorClass::RateLimit(_))),
        500 => assert!(matches!(class, ErrorClass::InternalServer(_))),
        501 => assert!(matches!(class, ErrorClass::Api(_))),
        502 => assert!(matches!(class, ErrorClass::BadGateway(_))),
        503 => assert!(matches!(class, ErrorClass::ServiceUnavailable(_))),
        _ => panic!("unexpected status {status}"),
    }
    assert_eq!(class.metadata().status_code, status);
    let sent = server.requests().await;
    assert_eq!(sent.len(), 1);
}

#[test]
fn cheapest_returns_three_names_in_price_order() {
    let harness = Harness::with_models(&[("openai", &["pricey", "cheap", "middle", "extra"][..])]);
    harness.prices.chat("openai", "cheap", "0.01", "0.01");
    harness.prices.chat("openai", "middle", "0.02", "0.03");
    harness.prices.chat("openai", "extra", "0.05", "0.05");
    harness.prices.chat("openai", "pricey", "1", "1");
    let names = harness.backend().cheapest_chat_models("openai", 3);
    assert_eq!(
        names,
        vec![
            String::from("cheap"),
            String::from("middle"),
            String::from("extra")
        ]
    );
    assert_eq!(harness.prices.provider(), Some(String::from("openai")));
}

#[test]
fn cheapest_keeps_the_model_order_on_a_tie() {
    let harness = Harness::with_models(&[("openai", &["alpha", "beta"][..])]);
    harness.prices.chat("openai", "alpha", "0.1", "0.1");
    harness.prices.chat("openai", "beta", "0.1", "0.1");
    let names = harness.backend().cheapest_chat_models("openai", 3);
    assert_eq!(names, vec![String::from("alpha"), String::from("beta")]);
}

#[test]
fn cheapest_honors_a_smaller_count() {
    let harness = Harness::with_models(&[("openai", &["alpha", "beta"][..])]);
    harness.prices.chat("openai", "alpha", "0.1", "0.1");
    harness.prices.chat("openai", "beta", "0.2", "0.2");
    assert_eq!(
        harness.backend().cheapest_chat_models("openai", 1),
        vec![String::from("alpha")]
    );
}

#[test]
fn cheapest_is_empty_for_an_unknown_provider() {
    let harness = Harness::new();
    assert!(harness.backend().cheapest_chat_models("nope", 3).is_empty());
}

#[test]
fn cheapest_is_empty_without_a_chat_model() {
    let harness = Harness::with_models(&[("openai", &["text-embedding-3-small"][..])]);
    harness.prices.embedding("openai", "text-embedding-3-small");
    assert!(
        harness
            .backend()
            .cheapest_chat_models("openai", 3)
            .is_empty()
    );
}

#[tokio::test]
async fn wiring_entry_probes_a_chat_model() {
    let harness = Harness::new();
    harness.chat.answer(&HEALTHY_HEADERS);
    let entry = harness.entry("gpt-4o", "openai");
    let outcome = entry
        .check_health(
            params(&[("model", text("openai/gpt-4o"))]),
            Some(String::from("chat")),
            None,
            None,
        )
        .await;
    let response = healthy(outcome);
    assert_eq!(
        response,
        health_check_response(&headers_map(&HEALTHY_HEADERS))
    );
    let call = harness.chat.take();
    assert_eq!(call.request.get("model"), Some(&text("openai/gpt-4o")));
    assert_eq!(
        call.request.get("cache"),
        Some(&json(r#"{"no-cache":true}"#))
    );
}

#[tokio::test]
async fn wiring_entry_probes_a_wildcard_model() {
    let harness = Harness::with_models(&[(
        "openai",
        &["cheap-model", "second-model", "third-model"][..],
    )]);
    harness.prices.chat("openai", "cheap-model", "0.01", "0.01");
    harness
        .prices
        .chat("openai", "second-model", "0.02", "0.02");
    harness.prices.chat("openai", "third-model", "0.03", "0.03");
    let entry = harness.entry("openai/*", "openai");
    let outcome = entry
        .check_health(params(&[("model", text("openai/*"))]), None, None, None)
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.chat.take();
    assert_eq!(call.request.get("model"), Some(&text("cheap-model")));
    assert_eq!(
        call.request.get("fallbacks"),
        Some(&json(r#"["second-model","third-model"]"#))
    );
    let Some(Json::Number(max_tokens)) = call.request.get("max_tokens") else {
        panic!("the wildcard call caps max_tokens");
    };
    assert_eq!(max_tokens.as_u64(), Some(16));
}

#[tokio::test]
async fn wiring_entry_probes_an_ocr_model_through_the_default_document() {
    let harness = Harness::new();
    let entry = harness.entry("mistral-ocr-latest", "mistral");
    let outcome = entry
        .check_health(
            params(&[("model", text("mistral-ocr-latest"))]),
            Some(String::from("ocr")),
            None,
            None,
        )
        .await;
    let _response = healthy(outcome);
    let recorded = harness.owner.take();
    assert_eq!(recorded.mode_name(), "ocr");
    let OwnerCall::Ocr {
        document: served, ..
    } = recorded
    else {
        panic!("the owner records an ocr call");
    };
    assert!(served.source().starts_with("data:application/pdf;base64,"));
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::mem;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageParams, ImageResponse, ImageUsage,
};
use betterlite_openai::error::ProviderError;
use secrecy::ExposeSecret as _;
use serde_json::Number;
use thiserror::Error;

use super::error::ImageGenerationError;
use super::log::{ImageGenerationLogger, PreCallRecord};
use super::{
    ClientKind, ImageGenerationCall, ImageGenerationDeps, ImageGenerationOutcome, SuppliedClient,
    prepare, send,
};
use crate::api::image_contracts::error::ImageContractError;
use crate::api::image_contracts::generation::{
    BaseImageGenerationConfig, ImageGenerationReply, ImageGenerationRequest,
};
use crate::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest};

mod support;

use support::{
    Captured, FailingSource, PoolSource, StubClient, StubSource, listener, reply, serve_once,
    stall_once,
};

const IMAGE_REPLY: &[u8] = br#"{"created":1,"data":[{"b64_json":"YQ=="}]}"#;

fn header_map(pairs: &[(&str, &str)]) -> ImageHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn fields(pairs: &[(&str, Json)]) -> ImageParams {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn litellm_params(api_base: &str, api_key: Option<&str>, ssl_verify: Option<bool>) -> ImageParams {
    let mut params = ImageParams::new();
    params.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    if let Some(key) = api_key {
        params.insert(String::from("api_key"), Json::Text(key.to_owned()));
    }
    if let Some(setting) = ssl_verify {
        params.insert(String::from("ssl_verify"), Json::Bool(setting));
    }
    params
}

fn caller_options(extra: Option<&ImageHeaders>) -> ImageParams {
    let mut params = ImageParams::new();
    if let Some(headers) = extra {
        let fields: ImageParams = headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        params.insert(String::from("extra_headers"), Json::Object(fields));
    }
    params
}

const fn generation<'call>(
    options: &'call ImageParams,
    params: &'call ImageParams,
) -> ImageGenerationCall<'call> {
    ImageGenerationCall {
        model: "image-model",
        prompt: "cat",
        optional_params: options,
        litellm_params: params,
        extra_headers: None,
        extra_body: None,
        fake_stream: false,
        litellm_metadata: None,
        api_key: None,
        timeout: Duration::from_secs(5),
    }
}

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] ImageContractError),
    #[error("the fixture refused the step")]
    Refused,
    #[error("the reply body is not the fixture's JSON")]
    Reply(#[source] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Work,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    Base,
    Broken,
}

#[derive(Default)]
struct Recorded {
    validate_api_key: Option<String>,
    validate_headers: Option<ImageHeaders>,
    validate_messages: Option<usize>,
    url_api_key: Option<String>,
    transform_headers: Option<ImageHeaders>,
    request_data: Option<ImageParams>,
}

// A provider config with no provider behaviour of its own: what it is handed is what it records
struct Fixture {
    validated: ImageHeaders,
    echo: ImageParams,
    endpoint: Endpoint,
    request: Step,
    response: Step,
    multipart: bool,
    recorded: Mutex<Recorded>,
}

impl Fixture {
    fn working(validated: ImageHeaders) -> Self {
        Self {
            validated,
            echo: ImageParams::new(),
            endpoint: Endpoint::Base,
            request: Step::Work,
            response: Step::Work,
            multipart: false,
            recorded: Mutex::new(Recorded::default()),
        }
    }

    fn recorded(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record_headers(&self, headers: &ImageHeaders) {
        let mut recorded = self.recorded();
        recorded.transform_headers = Some(headers.clone());
    }
}

impl BaseImageGenerationConfig for Fixture {
    type Error = FixtureError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _non_default_params: &ImageParams,
        optional_params: ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        optional_params
    }

    fn get_complete_url(&self, request: ImageUrlRequest<'_>) -> String {
        let key = request.api_key.map(|key| key.expose_secret().to_owned());
        {
            let mut recorded = self.recorded();
            recorded.url_api_key = key;
        }
        match self.endpoint {
            Endpoint::Base => request.api_base.unwrap_or_default().to_owned(),
            Endpoint::Broken => String::from("://not a URL"),
        }
    }

    fn validate_environment(&self, request: ImageEnvironment<'_>) -> ImageHeaders {
        {
            let mut recorded = self.recorded();
            recorded.validate_api_key = request.api_key.map(|key| key.expose_secret().to_owned());
            recorded.validate_headers = Some(request.headers.clone());
            recorded.validate_messages = Some(request.messages.len());
        }
        self.validated.clone()
    }

    fn transform_image_generation_request(
        &self,
        request: ImageGenerationRequest<'_>,
    ) -> Result<ImageParams, FixtureError> {
        match self.request {
            Step::Fail => Err(FixtureError::Refused),
            Step::Work => {
                let mut data = ImageParams::new();
                data.insert(String::from("model"), Json::Text(request.model.to_owned()));
                data.insert(
                    String::from("prompt"),
                    Json::Text(request.prompt.to_owned()),
                );
                data.extend(self.echo.clone());
                self.record_headers(request.headers);
                Ok(data)
            }
        }
    }

    fn transform_image_generation_response(
        &self,
        reply: ImageGenerationReply<'_>,
    ) -> Result<ImageResponse, FixtureError> {
        match self.response {
            Step::Fail => Err(FixtureError::Refused),
            Step::Work => {
                let parsed: ImageResponse = serde_json::from_slice(&reply.raw_response.body)
                    .map_err(FixtureError::Reply)?;
                {
                    let mut recorded = self.recorded();
                    recorded.request_data = Some(reply.request_data.clone());
                }
                reply.model_response.created = parsed.created;
                reply.model_response.data = parsed.data;
                Ok(mem::take(reply.model_response))
            }
        }
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }
}

#[derive(Default)]
struct Recorder {
    input: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    headers: Option<ImageHeaders>,
    complete_input_dict: Option<ImageParams>,
}

impl ImageGenerationLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord<'_>) {
        self.input = Some(record.input.to_owned());
        self.api_key = Some(record.api_key.to_owned());
        self.api_base = Some(record.api_base.to_owned());
        self.headers = Some(record.headers.clone());
        self.complete_input_dict = Some(record.complete_input_dict.clone());
    }
}

struct JsonRun {
    captured: Captured,
    outcome: ImageGenerationOutcome,
    fixture: Fixture,
    recorder: Recorder,
}

async fn json_generation() -> JsonRun {
    let (listener, base) = listener().await;
    let fixture = Fixture::working(header_map(&[("x", "validated")]));
    let params = litellm_params(&base, Some("stored"), None);
    let options = caller_options(Some(&header_map(&[("x", "option")])));
    let extra_headers = header_map(&[("x", "explicit")]);
    let extra_body = fields(&[("ignored", Json::Bool(true))]);
    let metadata = fields(&[("tag", Json::Text(String::from("t")))]);
    let mut call = generation(&options, &params);
    call.api_key = Some("direct");
    call.extra_headers = Some(&extra_headers);
    call.extra_body = Some(&extra_body);
    call.fake_stream = true;
    call.litellm_metadata = Some(&metadata);
    let mut recorder = Recorder::default();
    let source = PoolSource::new();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: None,
    };
    let prepared =
        prepare(&deps, ClientKind::Sync, &call, &mut recorder).expect("the prepared request");
    let served = reply(
        "200 OK",
        &[
            ("content-type", b"application/json"),
            ("x-request-id", b"img_1"),
        ],
        IMAGE_REPLY,
    );
    let (outcome, captured) = tokio::join!(send(prepared), serve_once(&listener, &served));
    JsonRun {
        captured,
        outcome: outcome.expect("the image response"),
        fixture,
        recorder,
    }
}

#[tokio::test]
async fn generation_json_transport() {
    let run = json_generation().await;
    assert_json_request(&run.captured);
    assert_json_response(&run.outcome);
    assert_json_effects(&run.fixture, &run.recorder);
}

// The body is the request transform's dict alone: extra_body is accepted by the handler and never merged
fn assert_json_request(captured: &Captured) {
    assert_eq!(captured.request_line(), "POST /v1 HTTP/1.1");
    assert_eq!(captured.header("x").as_deref(), Some("explicit"));
    assert_eq!(
        captured.header("content-type").as_deref(),
        Some("application/json")
    );
    let sent: ImageParams = serde_json::from_slice(captured.body()).expect("the JSON body");
    let expected = fields(&[
        ("model", Json::Text(String::from("image-model"))),
        ("prompt", Json::Text(String::from("cat"))),
    ]);
    assert_eq!(sent, expected);
}

fn assert_json_response(outcome: &ImageGenerationOutcome) {
    assert_eq!(outcome.response.created, 1);
    assert_eq!(
        outcome.response.data,
        Some(vec![ImageData {
            b64_json: Some(String::from("YQ==")),
            revised_prompt: None,
            url: None,
        }])
    );
    assert_eq!(outcome.response.usage, Some(ImageUsage::default()));
    let projection = &outcome.provider_headers;
    assert_eq!(
        projection.headers.get("x-request-id").map(String::as_str),
        Some("img_1")
    );
    let renamed = Json::Text(String::from("img_1"));
    assert_eq!(
        projection
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&renamed)
    );
}

// The handler hands the explicit key to validation and the stored one to the URL, and only extra_headers overlays
fn assert_json_effects(fixture: &Fixture, recorder: &Recorder) {
    let recorded = fixture.recorded();
    assert_eq!(recorded.validate_api_key.as_deref(), Some("direct"));
    assert_eq!(
        recorded.validate_headers,
        Some(header_map(&[("x", "option")]))
    );
    assert_eq!(recorded.validate_messages, Some(0));
    assert_eq!(recorded.url_api_key.as_deref(), Some("stored"));
    assert_eq!(
        recorded.transform_headers,
        Some(header_map(&[("x", "explicit")]))
    );
    assert_eq!(recorder.api_key.as_deref(), Some(""));
    assert_eq!(recorder.input.as_deref(), Some("cat"));
    assert_eq!(recorder.headers, Some(header_map(&[("x", "explicit")])));
    let expected = fields(&[
        ("model", Json::Text(String::from("image-model"))),
        ("prompt", Json::Text(String::from("cat"))),
    ]);
    assert_eq!(recorder.complete_input_dict, Some(expected));
}

fn multipart_fixture() -> Fixture {
    let mut fixture = Fixture::working(header_map(&[("x", "validated")]));
    fixture.multipart = true;
    fixture
}

async fn multipart_send(fixture: Fixture, kind: ClientKind) -> Captured {
    let (listener, base) = listener().await;
    let params = litellm_params(&base, None, None);
    let options = caller_options(None);
    let call = generation(&options, &params);
    let mut recorder = Recorder::default();
    let source = PoolSource::new();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: None,
    };
    let prepared = prepare(&deps, kind, &call, &mut recorder).expect("the prepared request");
    let served = reply(
        "200 OK",
        &[("content-type", b"application/json")],
        IMAGE_REPLY,
    );
    let (outcome, captured) = tokio::join!(send(prepared), serve_once(&listener, &served));
    outcome.expect("the image response");
    captured
}

#[tokio::test]
async fn generation_multipart_transport() {
    let sync_form = multipart_send(multipart_fixture(), ClientKind::Sync).await;
    let async_form = multipart_send(multipart_fixture(), ClientKind::Asynchronous).await;
    assert_multipart_form(&sync_form);
    assert_multipart_form(&async_form);
    assert_eq!(sync_form.normalized_body(), async_form.normalized_body());
    assert_eq!(sync_form.header("x").as_deref(), Some("validated"));
    supplied_client_serves_its_kind().await;
    wrong_kind_uses_the_shared_client().await;
    ssl_verify_reaches_the_source().await;
}

fn assert_multipart_form(captured: &Captured) {
    let content_type = captured.header("content-type").expect("the content type");
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    let body = captured.normalized_body();
    assert!(body.contains("Content-Disposition: form-data; name=\"model\"\r\n\r\nimage-model\r\n"));
    assert!(body.contains("Content-Disposition: form-data; name=\"prompt\"\r\n\r\ncat\r\n"));
    assert!(body.ends_with(
        "Content-Disposition: form-data; name=\"none\"; filename=\"upload\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--<boundary>--\r\n"
    ));
}

async fn supplied_client_serves_its_kind() {
    let supplied = StubClient::new(200, IMAGE_REPLY);
    let source = StubSource::new(StubClient::new(200, IMAGE_REPLY));
    let fixture = Fixture::working(ImageHeaders::new());
    let params = litellm_params("http://unused/v1", None, None);
    let options = caller_options(None);
    let call = generation(&options, &params);
    let mut recorder = Recorder::default();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
    };
    let prepared =
        prepare(&deps, ClientKind::Sync, &call, &mut recorder).expect("the prepared request");
    let outcome = send(prepared).await.expect("the stub response");
    assert_eq!(supplied.sent(), 1);
    assert!(source.seen().is_empty());
    assert_eq!(outcome.response.created, 1);
}

async fn wrong_kind_uses_the_shared_client() {
    let supplied = StubClient::new(200, IMAGE_REPLY);
    let source = StubSource::new(StubClient::new(200, IMAGE_REPLY));
    let fixture = Fixture::working(ImageHeaders::new());
    let params = litellm_params("http://unused/v1", None, None);
    let options = caller_options(None);
    let call = generation(&options, &params);
    let mut recorder = Recorder::default();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
    };
    let prepared = prepare(&deps, ClientKind::Asynchronous, &call, &mut recorder)
        .expect("the prepared request");
    send(prepared).await.expect("the shared client response");
    assert_eq!(supplied.sent(), 0);
    assert_eq!(source.stub().sent(), 1);
    assert_eq!(source.seen(), vec![None]);
}

async fn ssl_verify_reaches_the_source() {
    let source = StubSource::new(StubClient::new(200, IMAGE_REPLY));
    let fixture = Fixture::working(ImageHeaders::new());
    let params = litellm_params("http://unused/v1", None, Some(false));
    let options = caller_options(None);
    let call = generation(&options, &params);
    let mut recorder = Recorder::default();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: None,
    };
    let prepared =
        prepare(&deps, ClientKind::Sync, &call, &mut recorder).expect("the prepared request");
    send(prepared).await.expect("the shared client response");
    assert_eq!(source.seen(), vec![Some(false)]);
}

#[tokio::test]
async fn generation_multipart_fields() {
    let mut fixture = Fixture::working(ImageHeaders::new());
    fixture.multipart = true;
    fixture.echo = fields(&[
        ("flag", Json::Bool(true)),
        ("empty", Json::Null),
        ("quality", Json::Number(Number::from(80_u8))),
        (
            "style",
            Json::List(vec![
                Json::Text(String::from("a")),
                Json::Text(String::from("b")),
            ]),
        ),
    ]);
    let captured = multipart_send(fixture, ClientKind::Sync).await;
    let body = captured.normalized_body();
    assert!(body.contains("name=\"flag\"\r\n\r\ntrue\r\n"));
    assert!(body.contains("name=\"empty\"\r\n\r\n\r\n"));
    assert!(body.contains("name=\"quality\"\r\n\r\n80\r\n"));
    assert!(body.contains("name=\"style\"\r\n\r\na\r\n"));
    assert!(body.contains("name=\"style\"\r\n\r\nb\r\n"));
}

struct FailureRun {
    outcome: Result<ImageGenerationOutcome, ImageGenerationError>,
    captured: Captured,
    second_request: bool,
}

async fn failing_send(fixture: &Fixture, served: &[u8]) -> FailureRun {
    let (listener, base) = listener().await;
    let params = litellm_params(&base, None, None);
    let options = caller_options(None);
    let call = generation(&options, &params);
    let mut recorder = Recorder::default();
    let source = PoolSource::new();
    let deps = ImageGenerationDeps {
        config: fixture,
        source: &source,
        supplied: None,
    };
    let prepared =
        prepare(&deps, ClientKind::Sync, &call, &mut recorder).expect("the prepared request");
    let (outcome, captured) = tokio::join!(send(prepared), serve_once(&listener, served));
    let second_request = tokio::time::timeout(Duration::from_millis(50), listener.accept())
        .await
        .is_ok();
    FailureRun {
        outcome,
        captured,
        second_request,
    }
}

fn expect_provider(
    outcome: Result<ImageGenerationOutcome, ImageGenerationError>,
) -> Box<ProviderError> {
    match outcome {
        Err(ImageGenerationError::Provider(error)) => error,
        Err(other) => panic!("the send must fail as a provider error, got {other}"),
        Ok(_) => panic!("the send must fail"),
    }
}

#[tokio::test]
async fn generation_send_failure() {
    assert_status_error().await;
    assert_boundary_status().await;
    assert_unreadable_body().await;
    assert_response_skipped_after_status().await;
    assert_response_failure().await;
    assert_field_failures();
    assert_assembly_failures();
    assert_response_header_failure().await;
}

async fn assert_status_error() {
    let fixture = Fixture::working(ImageHeaders::new());
    let served = reply(
        "429 Too Many Requests",
        &[("content-type", b"text/plain"), ("retry-after", b"2")],
        b"slow",
    );
    let run = failing_send(&fixture, &served).await;
    assert!(!run.second_request);
    assert_eq!(run.captured.request_line(), "POST /v1 HTTP/1.1");
    let error = expect_provider(run.outcome);
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow");
    let headers = error.headers.expect("the masked headers");
    let expected = vec![
        (String::from("connection"), String::from("close")),
        (String::from("content-length"), String::from("4")),
        (String::from("content-type"), String::from("text/plain")),
        (String::from("retry-after"), String::from("2")),
    ];
    assert_eq!(headers, expected);
}

// An empty body leaves no content-length, since the recomputed one is only added for a body that has bytes
async fn assert_boundary_status() {
    let fixture = Fixture::working(ImageHeaders::new());
    let served = reply("400 Bad Request", &[], b"");
    let run = failing_send(&fixture, &served).await;
    let error = expect_provider(run.outcome);
    assert_eq!(error.status_code, 400);
    assert_eq!(error.message, "");
    assert_eq!(
        error.headers,
        Some(vec![(String::from("connection"), String::from("close"))])
    );
}

async fn assert_unreadable_body() {
    let fixture = Fixture::working(ImageHeaders::new());
    let served = reply("500 Internal Server Error", &[], b"\xff\xfe");
    let run = failing_send(&fixture, &served).await;
    let error = expect_provider(run.outcome);
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
}

// The request transform ran, so a response transform after the failed send would have recorded its data
async fn assert_response_skipped_after_status() {
    let fixture = Fixture::working(ImageHeaders::new());
    let served = reply("429 Too Many Requests", &[], b"rate");
    let run = failing_send(&fixture, &served).await;
    expect_provider(run.outcome);
    let recorded = fixture.recorded();
    assert!(recorded.transform_headers.is_some());
    assert_eq!(recorded.request_data, None);
}

async fn assert_response_failure() {
    let mut fixture = Fixture::working(ImageHeaders::new());
    fixture.response = Step::Fail;
    let served = reply(
        "200 OK",
        &[("content-type", b"application/json")],
        IMAGE_REPLY,
    );
    let run = failing_send(&fixture, &served).await;
    let error = run.outcome.expect_err("the transform failure");
    assert!(matches!(error, ImageGenerationError::Config(_)));
}

// A header the transport cannot carry, or a body value a form field cannot hold, fails before any send
async fn assert_response_header_failure() {
    let fixture = Fixture::working(ImageHeaders::new());
    let served = reply("200 OK", &[("x-opaque", b"\xff")], IMAGE_REPLY);
    let run = failing_send(&fixture, &served).await;
    match run.outcome.expect_err("the response header failure") {
        ImageGenerationError::HeaderText { name } => assert_eq!(name, "x-opaque"),
        other => panic!("expected the header text failure, got {other}"),
    }
}

fn prepare_error(
    fixture: &Fixture,
    params: &ImageParams,
    options: &ImageParams,
) -> Result<(), ImageGenerationError> {
    let call = generation(options, params);
    let mut recorder = Recorder::default();
    let source = PoolSource::new();
    let deps = ImageGenerationDeps {
        config: fixture,
        source: &source,
        supplied: None,
    };
    match prepare(&deps, ClientKind::Sync, &call, &mut recorder) {
        Ok(_prepared) => Ok(()),
        Err(error) => Err(error),
    }
}

fn field_error(fixture: &Fixture, params: &ImageParams) -> String {
    let options = caller_options(None);
    match prepare_error(fixture, params, &options).expect_err("the field type failure") {
        ImageGenerationError::FieldType { field, .. } => field.to_owned(),
        other => panic!("expected a field type error, got {other}"),
    }
}

fn assert_field_failures() {
    let fixture = Fixture::working(ImageHeaders::new());
    let good = litellm_params("http://127.0.0.1:9/v1", None, None);
    prepare_error(&fixture, &good, &caller_options(None))
        .expect("the well-formed request to prepare");
    let mut wrong_ssl = good.clone();
    wrong_ssl.insert(String::from("ssl_verify"), Json::Text(String::from("yes")));
    assert_eq!(
        field_error(&fixture, &wrong_ssl),
        "litellm_params.ssl_verify"
    );
    let mut wrong_key = good.clone();
    wrong_key.insert(String::from("api_key"), Json::Bool(true));
    assert_eq!(field_error(&fixture, &wrong_key), "litellm_params.api_key");
    let mut wrong_base = good.clone();
    wrong_base.insert(String::from("api_base"), Json::Number(Number::from(80_u8)));
    assert_eq!(
        field_error(&fixture, &wrong_base),
        "litellm_params.api_base"
    );
    let mut text_options = caller_options(None);
    text_options.insert(String::from("extra_headers"), Json::Text(String::from("x")));
    let error =
        prepare_error(&fixture, &good, &text_options).expect_err("the headers type failure");
    assert!(matches!(error, ImageGenerationError::HeadersType));
    let mut value_options = caller_options(None);
    let bad = fields(&[("x", Json::Bool(true))]);
    value_options.insert(String::from("extra_headers"), Json::Object(bad));
    let error =
        prepare_error(&fixture, &good, &value_options).expect_err("the header type failure");
    match error {
        ImageGenerationError::HeaderType { name } => assert_eq!(name, "x"),
        other => panic!("expected a header value error, got {other}"),
    }
}

fn assert_assembly_failures() {
    let good = litellm_params("http://127.0.0.1:9/v1", None, None);
    let mut failing = Fixture::working(ImageHeaders::new());
    failing.request = Step::Fail;
    let error = prepare_error(&failing, &good, &caller_options(None))
        .expect_err("the transform request failure");
    assert!(matches!(error, ImageGenerationError::Config(_)));
    let mut broken = Fixture::working(ImageHeaders::new());
    broken.endpoint = Endpoint::Broken;
    let error = prepare_error(&broken, &good, &caller_options(None)).expect_err("the URL failure");
    assert!(matches!(error, ImageGenerationError::RequestUrl { .. }));
    let bad_name = Fixture::working(header_map(&[("bad name", "value")]));
    let error = prepare_error(&bad_name, &good, &caller_options(None))
        .expect_err("the header name failure");
    assert!(matches!(error, ImageGenerationError::HeaderName { .. }));
    let bad_value = Fixture::working(header_map(&[("x", "line\nbreak")]));
    let error = prepare_error(&bad_value, &good, &caller_options(None))
        .expect_err("the header value failure");
    assert!(matches!(error, ImageGenerationError::HeaderValue { .. }));
    let fixture = Fixture::working(ImageHeaders::new());
    let source = FailingSource;
    let options = caller_options(None);
    let call = generation(&options, &good);
    let mut recorder = Recorder::default();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: None,
    };
    let error =
        prepare(&deps, ClientKind::Sync, &call, &mut recorder).expect_err("the client failure");
    assert!(matches!(error, ImageGenerationError::Client(_)));
}

#[tokio::test]
async fn generation_timeout_failure() {
    let sync = timeout_message(ClientKind::Sync, Duration::from_secs(1)).await;
    assert_eq!(
        sync,
        "litellm.Timeout: Connection timed out after 1.0 seconds."
    );
    let asynchronous = timeout_message(ClientKind::Asynchronous, Duration::from_millis(50)).await;
    assert!(
        asynchronous.starts_with(
            "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken=0."
        )
    );
    assert!(asynchronous.ends_with(" seconds"));
}

async fn timeout_message(kind: ClientKind, timeout: Duration) -> String {
    let (listener, base) = listener().await;
    let fixture = Fixture::working(ImageHeaders::new());
    let params = litellm_params(&base, None, None);
    let options = caller_options(None);
    let mut call = generation(&options, &params);
    call.timeout = timeout;
    let mut recorder = Recorder::default();
    let source = PoolSource::new();
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &source,
        supplied: None,
    };
    let prepared = prepare(&deps, kind, &call, &mut recorder).expect("the prepared request");
    let hold = timeout.saturating_add(Duration::from_millis(200));
    let (outcome, ()) = tokio::join!(send(prepared), stall_once(&listener, hold));
    let error = expect_provider(outcome);
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    error.message
}

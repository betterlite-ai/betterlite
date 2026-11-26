#![cfg(test)]

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::Method;
use rstest::rstest;

use super::support::{Bench, FakeSource, FakeTransport, Outcome, headers, params, redactor, text};
use crate::api::video_generation_call::error::{VideoGenerationCallError, failure, handler_error};
use crate::api::video_generation_call::handler::{
    GenerationHandlerEnvironment, GenerationHandlerRequest, TransportGenerationHandler,
    VideoGenerationHandling as _,
};
use crate::api::video_generation_call::testing::{TestingConfig, TestingError};
use crate::api::videos_http_transport::client::{ClientError, ClientKind, SuppliedClient};
use crate::api::videos_http_transport::error::{BadField, VideoTransportError};
use crate::api::videos_http_transport::video_config::{Headers, Params, VideoHttpConfig as _};

#[rstest]
#[case::sync(false, ClientKind::Sync)]
#[case::asynchronous(true, ClientKind::Asynchronous)]
#[tokio::test]
async fn adapter_preserves_the_request_and_provider_failure(
    #[case] is_async: bool,
    #[case] kind: ClientKind,
) {
    let mut bench = Bench::new(Outcome::NeverAnswers);
    let wire = FakeTransport {
        status: 429,
        requests: Mutex::new(Vec::new()),
    };
    let config = TestingConfig::default();
    let source = FakeSource::default();
    let env = bench.fixture.env.pre_post();
    let error = TransportGenerationHandler
        .handle(
            GenerationHandlerEnvironment {
                config: &config,
                source: &source,
                logging: &mut bench.fixture.state,
                env: &env,
            },
            handler_request(&wire, kind),
            is_async,
        )
        .await
        .unwrap_err();
    assert_rate_limit(error);
    assert!(
        source
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_wire(&wire);
    assert_transform(&config);
}

fn assert_rate_limit(error: VideoGenerationCallError) {
    assert_eq!(error.to_string(), "quota exhausted");
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    let ErrorClass::RateLimit(details) = &mapped.class else {
        panic!("429 must remain a rate limit");
    };
    assert_eq!(
        details.context.message,
        "RateLimitError: OpenAIException - quota exhausted"
    );
    let metadata = mapped.class.metadata();
    assert_eq!(metadata.status_code, 429);
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
    assert_eq!(metadata.model.as_deref(), Some("sora-2"));
}

fn handler_request(
    wire: &FakeTransport,
    kind: ClientKind,
) -> GenerationHandlerRequest<'_, FakeSource> {
    GenerationHandlerRequest {
        model: "sora-2".to_owned(),
        prompt: "a dancing bird".to_owned(),
        custom_llm_provider: "openai".to_owned(),
        litellm_params: params(&[
            ("api_base", text("https://video.test/create")),
            ("ssl_verify", Json::Bool(false)),
        ]),
        optional_params: params(&[("size", text("480x854"))]),
        extra_headers: Some(headers(&[("x-probe", "generation")])),
        extra_body: None,
        timeout: Duration::from_secs(17),
        client: Some(SuppliedClient {
            kind,
            transport: wire,
        }),
    }
}

fn assert_wire(wire: &FakeTransport) {
    let requests = wire.requests.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.url.as_str(), "https://video.test/create");
    assert_eq!(sent.timeout, Some(Duration::from_secs(17)));
    assert_eq!(sent.headers.get("x-probe").unwrap(), "generation");
    let RequestBody::Json(body) = &sent.body else {
        panic!("the config requests JSON");
    };
    let decoded: Params = serde_json::from_slice(body).unwrap();
    drop(requests);
    assert_eq!(decoded, params(&[("size", text("480x854"))]));
}

fn assert_transform(config: &TestingConfig) {
    let calls = config
        .creates
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.model, "sora-2");
    assert_eq!(call.prompt, "a dancing bird");
    assert_eq!(call.optional_params, params(&[("size", text("480x854"))]));
    assert_eq!(
        call.litellm_params,
        params(&[
            ("api_base", text("https://video.test/create")),
            ("ssl_verify", Json::Bool(false))
        ])
    );
    assert_eq!(call.headers, headers(&[("x-probe", "generation")]));
    drop(calls);
}

#[test]
fn config_facts_keep_the_status_and_message() {
    let original = VideoTransportError::Config(TestingError);
    let error = handler_error(&original, &TestingConfig::default());
    let VideoGenerationCallError::Transport {
        message,
        status_code,
        status_code_is_synthesized,
    } = &error
    else {
        panic!("config facts must reach exception dispatch");
    };
    assert_eq!(message, "the testing config only maps options");
    assert_eq!(*status_code, Some(401));
    assert!(!status_code_is_synthesized);
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    assert!(matches!(mapped.class, ErrorClass::Authentication(_)));
    assert_eq!(mapped.class.metadata().status_code, 401);
    assert_eq!(mapped.class.metadata().provider.as_deref(), Some("openai"));
}

#[rstest]
#[case::client(VideoTransportError::Client(ClientError::UnknownProvider { provider: "missing".to_owned() }), "\"missing\" is not a provider betterlite knows")]
#[case::logging(
    VideoTransportError::Logging(PrePostError::RawRequestCaptured),
    "the raw request was captured instead of sent"
)]
fn local_errors_keep_their_text(
    #[case] original: VideoTransportError<TestingError>,
    #[case] expected: &str,
) {
    let error = handler_error(&original, &TestingConfig::default());
    let VideoGenerationCallError::Transport {
        message,
        status_code,
        status_code_is_synthesized,
    } = &error
    else {
        panic!("local text must reach exception dispatch");
    };
    assert_eq!(message, expected);
    assert_eq!(*status_code, None);
    assert!(!status_code_is_synthesized);
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    assert_eq!(mapped.to_string(), expected);
    assert!(matches!(mapped.class, ErrorClass::ApiConnection(_)));
    assert_eq!(mapped.class.metadata().provider.as_deref(), Some("openai"));
}

#[test]
fn field_errors_keep_the_field_and_expected_type() {
    let original = VideoTransportError::Field(BadField {
        field: "extra_headers",
        expected: "a map of strings",
    });
    let error = handler_error(&original, &TestingConfig::default());
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    assert_eq!(mapped.to_string(), "extra_headers must be a map of strings");
    let ErrorClass::BadRequest(details) = &mapped.class else {
        panic!("bad fields answer 400");
    };
    assert_eq!(details.body_param.as_deref(), Some("extra_headers"));
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn adapter_selects_the_client_and_maps_config_failure() {
    let mut bench = Bench::new(Outcome::NeverAnswers);
    let wire = FakeTransport::default();
    let config = TestingConfig::default();
    let source = FakeSource::default();
    let env = bench.fixture.env.pre_post();
    let mut request = handler_request(&wire, ClientKind::Asynchronous);
    request.client = None;
    let error = TransportGenerationHandler
        .handle(
            GenerationHandlerEnvironment {
                config: &config,
                source: &source,
                logging: &mut bench.fixture.state,
                env: &env,
            },
            request,
            true,
        )
        .await
        .unwrap_err();
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    assert!(matches!(mapped.class, ErrorClass::Authentication(_)));
    assert_eq!(mapped.class.metadata().status_code, 401);
    assert_eq!(mapped.to_string(), "the testing config only maps options");
    let clients = source
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(
        *clients,
        vec![(ClientKind::Asynchronous, Some(false), "openai".to_owned())]
    );
    drop(clients);
    assert_transform(&config);
}

#[rstest]
#[case::provider(false)]
#[case::synthesized(true)]
fn provider_errors_preserve_the_status_origin(#[case] synthesized: bool) {
    let config = TestingConfig::default();
    let mut carried = config.get_error_class("upstream failed", 500, &Headers::new());
    carried.status_code_is_synthesized = synthesized;
    let original = VideoTransportError::Provider(Box::new(carried));
    let error = handler_error(&original, &config);
    let VideoGenerationCallError::Transport {
        message,
        status_code,
        status_code_is_synthesized,
    } = &error
    else {
        panic!("provider facts must reach dispatch");
    };
    assert_eq!(message, "upstream failed");
    assert_eq!(*status_code, Some(500));
    assert_eq!(*status_code_is_synthesized, synthesized);
    let mapped = failure(error, Some("sora-2"), Some("openai"), &redactor());
    assert!(matches!(mapped.class, ErrorClass::InternalServer(_)));
    assert_eq!(mapped.class.metadata().status_code, 500);
    assert_eq!(mapped.class.metadata().provider.as_deref(), Some("openai"));
}

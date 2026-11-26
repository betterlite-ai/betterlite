#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready as future_ready};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_core::api::video_generation_call::call::{
    PendingVideoGeneration, VideoGenerationCaller,
};
use betterlite_core::api::video_generation_call::error::{
    VideoGenerationCallError, VideoGenerationFailure,
};
use betterlite_core::api::video_generation_call::handler::{
    GenerationHandlerEnvironment, GenerationHandlerRequest, VideoGenerationHandling,
};
use betterlite_core::api::video_generation_call::providers::{
    VideoGenerationConfig, VideoGenerationProviders,
};
use betterlite_core::api::video_records::{VideoCreateOptionalRequestParams, VideoObject};
use betterlite_core::api::video_remix_call::config::DEFAULT_TIMEOUT_SECONDS;
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_core::api::videos_http_transport::client::{
    ClientError, ClientRequest, ClientSource, VideoRequest, VideoTransport,
};
use betterlite_core::api::videos_http_transport::video_config::{
    Headers, Params, RequestFiles, VideoHttpConfig, VideoLogging, VideoReply, VideoRequestContext,
};
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use thiserror::Error;

use super::{
    VideoGenerationProbe, VideoGenerationSetup, generation_reply, generation_request,
    restore_logging, take_logging,
};
use crate::health_native_backend::seams::VideoGenerationProbeCall as _;
use crate::health_probe_modes::calls::ProbeCall;
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::{HealthProbeLogging, ProbeLogging};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn probe(parameters: &[(&str, Json)]) -> ProbeCall {
    ProbeCall {
        parameters: parameters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
        api_key: None,
        logging: None,
    }
}

fn header(name: &str, value: &str) -> IndexMap<String, Json> {
    IndexMap::from([(name.to_owned(), text(value))])
}

fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").expect("the test time parses")
}

fn state(model: &str) -> LoggingState {
    let mut correlation = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut correlation,
        settings: CorrelationSettings::default(),
    };
    let init = LoggingInit {
        model: model.to_owned(),
        call_type: String::new(),
        start_time: fixed_time(),
        litellm_call_id: String::new(),
        function_id: String::new(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let redactor = Redactor::new(16).expect("the test redactor builds");
    LoggingState::new(init, setup, &redactor, String::new, || Duration::ZERO)
        .expect("the test logging state builds")
}

#[test]
fn generation_request_binds_the_named_text_arguments() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("custom_llm_provider", text("openai")),
        ("seconds", text("8")),
        ("size", text("720x1280")),
        ("user", text("user-1")),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(request.prompt, "a prompt");
    assert_eq!(request.model.as_deref(), Some("sora-2"));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.seconds.as_deref(), Some("8"));
    assert_eq!(request.size.as_deref(), Some("720x1280"));
    assert_eq!(request.user.as_deref(), Some("user-1"));
}

#[test]
fn generation_request_carries_the_extra_maps() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("extra_headers", Json::Object(header("x-team", "health"))),
        ("extra_query", Json::Object(header("cursor", "after"))),
        ("extra_body", Json::Object(header("size", "480x854"))),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(
        request.extra_headers,
        Some(Headers::from([("x-team".to_owned(), "health".to_owned())]))
    );
    assert_eq!(
        request.extra_query,
        Some(Params::from([("cursor".to_owned(), text("after"))]))
    );
    assert_eq!(
        request.extra_body,
        Some(Params::from([("size".to_owned(), text("480x854"))]))
    );
}

#[test]
fn generation_request_keeps_the_loose_keywords() {
    let mut call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("extra_headers", Json::Object(header("x-team", "health"))),
        ("api_base", text("https://api.openai.com/v1")),
        ("resolution", text("1080x1920")),
    ]);
    call.api_key = Some(SecretString::from("sk-test".to_owned()));
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(
        request.kwargs.get("api_base"),
        Some(&text("https://api.openai.com/v1"))
    );
    assert_eq!(request.kwargs.get("resolution"), Some(&text("1080x1920")));
    assert_eq!(request.kwargs.get("api_key"), Some(&text("sk-test")));
    assert!(!request.kwargs.contains_key("prompt"));
    assert!(!request.kwargs.contains_key("model"));
    assert!(!request.kwargs.contains_key("extra_headers"));
    assert!(!request.kwargs.contains_key("timeout"));
}

#[test]
fn generation_request_keeps_the_signature_timeout() {
    let absent = probe(&[("prompt", text("a prompt"))]);
    let request = generation_request(&absent).expect("the health arguments convert");
    assert_eq!(request.timeout, Some(DEFAULT_TIMEOUT_SECONDS));
    let null = probe(&[("prompt", text("a prompt")), ("timeout", Json::Null)]);
    let request = generation_request(&null).expect("the health arguments convert");
    assert_eq!(request.timeout, None);
}

#[test]
fn generation_request_takes_a_numeric_timeout() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("timeout", Json::Number(Number::from(30_u64))),
    ]);
    let request = generation_request(&call).expect("the health arguments convert");
    assert_eq!(request.timeout, Some(30.0_f64));
}

#[test]
fn generation_request_refuses_a_parameter_of_the_wrong_type() {
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("seconds", Json::Number(Number::from(8_u64))),
    ]);
    let Err(error) = generation_request(&call) else {
        panic!("a number cannot be seconds");
    };
    let HealthProbeError::ParameterValue { key, .. } = error else {
        panic!("the failure names the parameter it could not convert");
    };
    assert_eq!(key, "seconds");
}

#[test]
fn generation_request_requires_the_prompt() {
    let call = probe(&[("model", text("sora-2"))]);
    let Err(error) = generation_request(&call) else {
        panic!("a prompt is required");
    };
    let HealthProbeError::ParameterValue { key, .. } = error else {
        panic!("the failure names the parameter it could not convert");
    };
    assert_eq!(key, "prompt");
}

#[test]
fn generation_reply_carries_the_hidden_headers() {
    let mut object = VideoObject::new("video-1".to_owned(), "completed".to_owned());
    object.hidden_params.insert(
        "headers".to_owned(),
        Json::Object(header("x-ms-region", "westeurope")),
    );
    let reply = generation_reply(&object);
    assert_eq!(
        reply.hidden_headers,
        BTreeMap::from([(String::from("x-ms-region"), String::from("westeurope"))])
    );
}

#[test]
fn the_call_swaps_the_shared_logging_state_out_and_back() {
    let logging: Arc<dyn HealthProbeLogging> = Arc::new(ProbeLogging::from(state("stored")));
    let shared = logging
        .logging_state()
        .expect("the health object exposes its state");
    let mut call = probe(&[("prompt", text("a prompt"))]);
    call.logging = Some(logging);
    let (taken, restored) = take_logging(&call, state("placeholder"));
    assert_eq!(taken.model, "stored");
    assert!(Arc::ptr_eq(
        restored.as_ref().expect("the shared state is retained"),
        &shared
    ));
    let current = shared.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(current.model, "placeholder");
    drop(current);
    restore_logging(restored, state("updated"));
    let current = shared.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(current.model, "updated");
}

#[test]
fn a_call_without_a_logging_object_takes_the_placeholder() {
    let call = probe(&[("prompt", text("a prompt"))]);
    let (taken, restored) = take_logging(&call, state("placeholder"));
    assert_eq!(taken.model, "placeholder");
    assert!(restored.is_none());
}

// the production probe over a fake caller and handler, so generate's own wiring is what the test reads
fn video_probe(fails: bool) -> VideoGenerationProbe<FakeProviders, FakeHandler, FakeSetup> {
    VideoGenerationProbe {
        caller: VideoGenerationCaller {
            providers: FakeProviders {
                config: FakeConfig,
                source: FakeSource,
            },
            handler: FakeHandler { fails },
            redactor: Redactor::new(16).expect("the test redactor builds"),
            request_timeout: Duration::from_secs(600),
            drop_params: true,
        },
        setup: FakeSetup {
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        },
        clock: fixed_time,
    }
}

#[tokio::test]
async fn generate_answers_with_the_hidden_headers_of_the_object() {
    let owner = video_probe(false);
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("custom_llm_provider", text("openai")),
    ]);
    let reply = owner.generate(call).await.expect("the probe answers");
    assert_eq!(
        reply.hidden_headers,
        BTreeMap::from([(String::from("x-ms-region"), String::from("westeurope"))])
    );
}

#[tokio::test]
async fn generate_reports_a_handler_failure_as_the_backend_class() {
    let owner = video_probe(true);
    let call = probe(&[
        ("prompt", text("a prompt")),
        ("model", text("sora-2")),
        ("custom_llm_provider", text("openai")),
    ]);
    let error = owner.generate(call).await.expect_err("the handler fails");
    let HealthProbeError::Backend {
        class,
        message,
        source,
    } = error
    else {
        panic!("a handler failure keeps its class");
    };
    assert_eq!(message, "size must be a string");
    let original = source
        .downcast_ref::<VideoGenerationFailure>()
        .expect("the original generation failure remains the source");
    assert_eq!(original.to_string(), message);
    assert_eq!(original.class, class);
    let ErrorClass::BadRequest(_) = class else {
        panic!("a handler failure keeps its 400 class");
    };
}

struct FakeSetup {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl VideoGenerationSetup for FakeSetup {
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

struct FakeProviders {
    config: FakeConfig,
    source: FakeSource,
}

impl VideoGenerationProviders for FakeProviders {
    type Config = FakeConfig;
    type Source = FakeSource;

    fn video_config(&self, _tag: &VideoConfigTag) -> Option<(&FakeConfig, &FakeSource)> {
        Some((&self.config, &self.source))
    }
}

struct FakeHandler {
    fails: bool,
}

impl<Config, Source> VideoGenerationHandling<Config, Source> for FakeHandler
where
    Config: VideoGenerationConfig,
    Source: ClientSource,
{
    fn handle<'call>(
        &'call self,
        _environment: GenerationHandlerEnvironment<'call, '_, Config, Source>,
        _request: GenerationHandlerRequest<'call, Source>,
        _is_async: bool,
    ) -> PendingVideoGeneration<'call> {
        if self.fails {
            return Box::pin(
                future_ready::<Result<VideoObject, VideoGenerationCallError>>(Err(
                    VideoGenerationCallError::FieldType {
                        field: "size",
                        expected: "a string",
                    },
                )),
            );
        }
        let mut object = VideoObject::new(String::from("video-1"), String::from("completed"));
        object.hidden_params.insert(
            "headers".to_owned(),
            Json::Object(header("x-ms-region", "westeurope")),
        );
        Box::pin(future_ready::<Result<VideoObject, VideoGenerationCallError>>(Ok(object)))
    }
}

struct FakeSource;

impl ClientSource for FakeSource {
    type Client = FakeTransport;

    fn client(&self, _request: ClientRequest<'_>) -> Result<FakeTransport, ClientError> {
        Ok(FakeTransport)
    }
}

struct FakeTransport;

impl VideoTransport for FakeTransport {
    fn send(
        &self,
        _request: VideoRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        future_ready(Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }))
    }
}

#[derive(Debug, Error)]
#[error("the fake video config performs no transport work")]
struct FakeConfigError;

struct FakeConfig;

impl VideoGenerationConfig for FakeConfig {
    fn map_openai_params(
        &self,
        _options: &VideoCreateOptionalRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Params {
        Params::new()
    }
}

impl VideoHttpConfig for FakeConfig {
    type Error = FakeConfigError;

    fn get_complete_url(
        &self,
        _model: &str,
        _api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, FakeConfigError> {
        Ok(String::from("http://health.test/videos"))
    }

    fn transform_video_create_request(
        &self,
        _model: &str,
        _prompt: &str,
        _optional_params: &Params,
        _context: VideoRequestContext<'_>,
    ) -> Result<(Params, RequestFiles, String), FakeConfigError> {
        Err(FakeConfigError)
    }

    fn transform_video_create_response(
        &self,
        _model: &str,
        _reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, FakeConfigError> {
        Err(FakeConfigError)
    }

    fn transform_video_content_request(
        &self,
        _video_id: &str,
        _variant: Option<&str>,
        _context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), FakeConfigError> {
        Err(FakeConfigError)
    }

    fn transform_video_content_response(
        &self,
        _raw_response: &UpstreamResponse,
        _logging_obj: &mut VideoLogging,
    ) -> Result<Vec<u8>, FakeConfigError> {
        Err(FakeConfigError)
    }
}

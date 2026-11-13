#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::batch_provider_create::Settings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::{
    BatchEndpoint, BatchObject, CompletionWindow, OutputExpiry,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::BatchProviderCreateError;
use super::transports::BatchCreateTransport;
use super::{
    BatchOptionalParams, BatchProvider, BatchProviderCreate, BatchProviderCreateCall,
    BatchProviderCreateInput, DEFAULT_OPENAI_BASE, DEFAULT_TIMEOUT_SECONDS, HttpTimeout,
    RequestTimeout, VertexCredentials,
};
use crate::api::api_boundaries::ApiLimits;

struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Reject)
                .expect("an empty catalog parses"),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
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

#[derive(Clone, Default)]
struct RecordingTransport {
    calls: Arc<Mutex<Vec<BatchProviderCreateCall>>>,
}

impl BatchCreateTransport for RecordingTransport {
    fn create(
        &self,
        call: &BatchProviderCreateCall,
    ) -> Result<BatchObject, BatchProviderCreateError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        Ok(batch())
    }
}

fn batch() -> BatchObject {
    serde_json::from_str(
        r#"{"id":"batch-1","completion_window":"24h","created_at":0,"endpoint":"/v1/chat/completions","input_file_id":"file-1","object":"batch","status":"validating"}"#,
    )
    .expect("a batch object")
}

fn run(
    input: &BatchProviderCreateInput,
    settings: &Settings,
) -> (
    Result<super::BatchProviderCreateResult, BatchProviderCreateError>,
    Vec<BatchProviderCreateCall>,
) {
    let resolution = Resolution::new();
    let context = resolution.context();
    let transport = RecordingTransport::default();
    let calls = Arc::clone(&transport.calls);
    let service = BatchProviderCreate {
        settings: settings.clone(),
        resolution: &context,
        transport,
        limits: ApiLimits::default(),
    };
    let outcome = service.create(input);
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (outcome, recorded)
}

fn created(
    input: &BatchProviderCreateInput,
    settings: &Settings,
) -> super::BatchProviderCreateResult {
    let (outcome, calls) = run(input, settings);
    assert_eq!(calls.len(), 1, "the transport is called exactly once");
    outcome.expect("the batch is created")
}

fn input(provider: BatchProvider) -> BatchProviderCreateInput {
    BatchProviderCreateInput {
        completion_window: CompletionWindow::Day,
        endpoint: BatchEndpoint::ChatCompletions,
        input_file_id: "file-1".to_owned(),
        custom_llm_provider: provider,
        metadata: None,
        extra_headers: None,
        extra_body: None,
        output_expires_after: None,
        model: None,
        is_async: false,
        optional_params: BatchOptionalParams::default(),
    }
}

fn body_of(call: &BatchProviderCreateCall) -> &CreateBatchRequest {
    match call {
        BatchProviderCreateCall::ProviderConfig(call) => &call.body,
        BatchProviderCreateCall::Xai(call) => &call.body,
        BatchProviderCreateCall::OpenAiCompatible(call) => &call.body,
        BatchProviderCreateCall::Azure(call) => &call.body,
        BatchProviderCreateCall::VertexAi(call) => &call.body,
    }
}

fn revealed(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

#[derive(Clone, Copy, Debug)]
enum BodyCase {
    PreservesEveryCreateField,
    OutputExpiryIsOptional,
    MetadataKeepsItsOrder,
    ProviderConfigNeedsAModel,
    ProviderConfigCarriesExtraHeaders,
    LimitsBoundTheCreateMaps,
    VertexCredentialsAcceptFields,
    AsyncFlagReachesTheAdapter,
}

#[rstest]
#[case::preserves_every_create_field(BodyCase::PreservesEveryCreateField)]
#[case::output_expiry_is_optional(BodyCase::OutputExpiryIsOptional)]
#[case::metadata_keeps_its_order(BodyCase::MetadataKeepsItsOrder)]
#[case::provider_config_needs_a_model(BodyCase::ProviderConfigNeedsAModel)]
#[case::provider_config_carries_extra_headers(BodyCase::ProviderConfigCarriesExtraHeaders)]
#[case::limits_bound_the_create_maps(BodyCase::LimitsBoundTheCreateMaps)]
#[case::vertex_credentials_accept_fields(BodyCase::VertexCredentialsAcceptFields)]
#[case::async_flag_reaches_the_adapter(BodyCase::AsyncFlagReachesTheAdapter)]
fn batch_create_body(#[case] case: BodyCase) {
    match case {
        BodyCase::PreservesEveryCreateField => preserves_every_create_field(),
        BodyCase::OutputExpiryIsOptional => output_expiry_is_optional(),
        BodyCase::MetadataKeepsItsOrder => metadata_keeps_its_order(),
        BodyCase::ProviderConfigNeedsAModel => provider_config_needs_a_model(),
        BodyCase::ProviderConfigCarriesExtraHeaders => provider_config_carries_extra_headers(),
        BodyCase::LimitsBoundTheCreateMaps => limits_bound_the_create_maps(),
        BodyCase::VertexCredentialsAcceptFields => vertex_credentials_accept_fields(),
        BodyCase::AsyncFlagReachesTheAdapter => async_flag_reaches_the_adapter(),
    }
}

fn preserves_every_create_field() {
    let mut request = input(BatchProvider::Xai);
    request.metadata = Some(IndexMap::from([
        ("first".to_owned(), "one".to_owned()),
        ("second".to_owned(), "two".to_owned()),
    ]));
    request.extra_headers = Some(IndexMap::from([("x-a".to_owned(), "b".to_owned())]));
    request.extra_body = Some(IndexMap::from([("x-c".to_owned(), "d".to_owned())]));
    request.output_expires_after = Some(OutputExpiry {
        anchor: betterlite_openai::batches::records::ExpiryAnchor::CreatedAt,
        seconds: 60,
    });
    let result = created(&request, &Settings::default());
    let body = body_of(&result.call);
    assert_eq!(body.input_file_id.as_deref(), Some("file-1"));
    assert_eq!(body.endpoint, Some(BatchEndpoint::ChatCompletions));
    assert_eq!(body.completion_window, Some(CompletionWindow::Day));
    assert_eq!(
        body.metadata
            .as_ref()
            .and_then(|fields| fields.get("second")),
        Some(&"two".to_owned())
    );
    assert_eq!(
        body.extra_headers
            .as_ref()
            .and_then(|fields| fields.get("x-a")),
        Some(&"b".to_owned())
    );
    assert_eq!(
        body.extra_body
            .as_ref()
            .and_then(|fields| fields.get("x-c")),
        Some(&"d".to_owned())
    );
    assert_eq!(
        body.output_expires_after,
        Some(OutputExpiry {
            anchor: betterlite_openai::batches::records::ExpiryAnchor::CreatedAt,
            seconds: 60,
        })
    );
    assert_eq!(result.batch.id, "batch-1");
}

fn output_expiry_is_optional() {
    let result = created(&input(BatchProvider::Openai), &Settings::default());
    assert_eq!(body_of(&result.call).output_expires_after, None);
    let encoded = serde_json::to_string(body_of(&result.call)).expect("the body encodes");
    assert!(
        encoded.contains(r#""completion_window":"24h""#),
        "{encoded}"
    );
    assert!(!encoded.contains("output_expires_after"), "{encoded}");
}

fn metadata_keeps_its_order() {
    let mut request = input(BatchProvider::Xai);
    request.metadata = Some(IndexMap::from([
        ("zulu".to_owned(), "1".to_owned()),
        ("alpha".to_owned(), "2".to_owned()),
        ("mike".to_owned(), "3".to_owned()),
    ]));
    let result = created(&request, &Settings::default());
    let keys: Vec<&str> = body_of(&result.call)
        .metadata
        .as_ref()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .unwrap_or_default();
    assert_eq!(keys, vec!["zulu", "alpha", "mike"]);
}

fn provider_config_needs_a_model() {
    let (without_model, calls) = run(&input(BatchProvider::Bedrock), &Settings::default());
    assert!(matches!(
        without_model,
        Err(BatchProviderCreateError::UnsupportedProvider {
            provider: BatchProvider::Bedrock
        })
    ));
    assert!(calls.is_empty());
    let mut request = input(BatchProvider::Bedrock);
    request.model = Some("bedrock/anthropic.claude-3".to_owned());
    let result = created(&request, &Settings::default());
    let BatchProviderCreateCall::ProviderConfig(call) = &result.call else {
        panic!("a provider config owns the bedrock branch");
    };
    assert_eq!(call.provider, BatchProvider::Bedrock);
    assert_eq!(call.model, "anthropic.claude-3");
    assert!(call.headers.is_empty());
}

fn provider_config_carries_extra_headers() {
    let mut request = input(BatchProvider::Mistral);
    request.model = Some("mistral/mistral-large".to_owned());
    request.extra_headers = Some(IndexMap::from([(
        "x-custom".to_owned(),
        "value".to_owned(),
    )]));
    let result = created(&request, &Settings::default());
    let BatchProviderCreateCall::ProviderConfig(call) = &result.call else {
        panic!("a provider config owns the mistral branch");
    };
    assert_eq!(call.model, "mistral-large");
    assert_eq!(
        call.headers.get("x-custom").map(String::as_str),
        Some("value")
    );
}

fn limits_bound_the_create_maps() {
    let mut request = input(BatchProvider::Xai);
    request.metadata = Some(IndexMap::from([("first".to_owned(), "one".to_owned())]));
    request.extra_headers = Some(IndexMap::from([("x-a".to_owned(), "b".to_owned())]));
    request.extra_body = Some(IndexMap::from([("x-c".to_owned(), "d".to_owned())]));
    assert!(outcome_is_overflow(create_outcome(&request, 2), 3, 2));
    let mut wide = input(BatchProvider::Xai);
    wide.metadata = Some(IndexMap::from([
        ("first".to_owned(), "one".to_owned()),
        ("second".to_owned(), "two".to_owned()),
    ]));
    assert!(outcome_is_overflow(create_outcome(&wide, 1), 2, 1));
}

fn create_outcome(
    request: &BatchProviderCreateInput,
    max_items: usize,
) -> Result<super::BatchProviderCreateResult, BatchProviderCreateError> {
    let resolution = Resolution::new();
    let context = resolution.context();
    let service = BatchProviderCreate {
        settings: Settings::default(),
        resolution: &context,
        transport: RecordingTransport::default(),
        limits: ApiLimits {
            max_json_bytes: ApiLimits::default().max_json_bytes,
            max_event_bytes: ApiLimits::default().max_event_bytes,
            max_items,
        },
    };
    service.create(request)
}

fn outcome_is_overflow(
    outcome: Result<super::BatchProviderCreateResult, BatchProviderCreateError>,
    observed: usize,
    limit: usize,
) -> bool {
    matches!(
        outcome,
        Err(BatchProviderCreateError::Limit(
            crate::api::api_boundaries::ApiLimitError::TooManyItems {
                observed: seen,
                limit: maximum
            }
        )) if seen == observed && maximum == limit
    )
}

fn vertex_credentials_accept_fields() {
    let mut request = input(BatchProvider::VertexAi);
    request.optional_params.vertex_credentials =
        Some(VertexCredentials::Fields(BTreeMap::from([(
            "client_email".to_owned(),
            "svc@example.com".to_owned(),
        )])));
    let result = created(&request, &Settings::default());
    let BatchProviderCreateCall::VertexAi(call) = &result.call else {
        panic!("vertex owns its own branch");
    };
    let Some(VertexCredentials::Fields(fields)) = &call.vertex_credentials else {
        panic!("the decoded service-account fields survive");
    };
    assert_eq!(
        fields.get("client_email").map(String::as_str),
        Some("svc@example.com")
    );
}

fn async_flag_reaches_the_adapter() {
    let mut request = input(BatchProvider::Openai);
    request.is_async = true;
    let result = created(&request, &Settings::default());
    let BatchProviderCreateCall::OpenAiCompatible(call) = &result.call else {
        panic!("openai owns the compatible branch");
    };
    assert!(call.is_async);
}

#[rstest]
#[case::chat_completions(BatchEndpoint::ChatCompletions)]
#[case::embeddings(BatchEndpoint::Embeddings)]
#[case::completions(BatchEndpoint::Completions)]
#[case::responses(BatchEndpoint::Responses)]
#[case::ocr(BatchEndpoint::Ocr)]
#[case::image_generations(BatchEndpoint::ImageGenerations)]
#[case::image_edits(BatchEndpoint::ImageEdits)]
#[case::video_generations(BatchEndpoint::VideoGenerations)]
#[case::videos(BatchEndpoint::Videos)]
#[case::video_edits(BatchEndpoint::VideoEdits)]
#[case::video_extensions(BatchEndpoint::VideoExtensions)]
fn completion_window(#[case] endpoint: BatchEndpoint) {
    let mut request = input(BatchProvider::Openai);
    request.endpoint = endpoint;
    let result = created(&request, &Settings::default());
    let body = body_of(&result.call);
    assert_eq!(body.completion_window, Some(CompletionWindow::Day));
    assert_eq!(body.endpoint, Some(endpoint));
    let encoded = serde_json::to_string(body).expect("the body encodes");
    assert!(
        encoded.contains(r#""completion_window":"24h""#),
        "{encoded}"
    );
    assert!(encoded.contains(&endpoint_wire(endpoint)), "{encoded}");
}

fn endpoint_wire(endpoint: BatchEndpoint) -> String {
    let encoded = serde_json::to_string(&endpoint).expect("the endpoint encodes");
    format!(r#""endpoint":{encoded}"#)
}

#[rstest]
#[case::twelve_hours(r#""12h""#)]
#[case::one_hour(r#""1h""#)]
#[case::number("24")]
#[case::null("null")]
fn completion_window_rejects_other_values(#[case] window: &str) {
    let source = format!(
        r#"{{"completion_window":{window},"endpoint":"/v1/chat/completions","input_file_id":"file-1"}}"#
    );
    let mut deserializer = serde_json::Deserializer::from_str(&source);
    let decoded: Result<BatchProviderCreateInput, _> =
        serde_path_to_error::deserialize(&mut deserializer);
    let error = decoded.err().expect("a window other than 24h is rejected");
    assert_eq!(error.path().to_string(), "completion_window");
}

#[rstest]
#[case::optional_seconds_win(Some(RequestTimeout::Seconds(12.5)), Some(7.5), 12.5)]
#[case::request_timeout_fills_in(None, Some(7.5), 7.5)]
#[case::default_when_both_absent(None, None, DEFAULT_TIMEOUT_SECONDS)]
#[case::explicit_zero_stays_zero(Some(RequestTimeout::Seconds(0.0)), Some(7.5), 0.0)]
#[case::request_timeout_zero_stays_zero(None, Some(0.0), 0.0)]
#[case::client_read_value(client_with_read(Some(5.0)), None, 5.0)]
#[case::client_without_read_takes_the_default(client_with_read(None), None, 600.0)]
#[case::client_zero_read_stays_zero(client_with_read(Some(0.0)), None, 0.0)]
#[case::client_beats_request_timeout(client_with_read(Some(9.0)), Some(7.5), 9.0)]
fn timeout_precedence(
    #[case] optional: Option<RequestTimeout>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: f64,
) {
    let mut request = input(BatchProvider::Xai);
    request.optional_params.timeout = optional;
    request.optional_params.request_timeout = request_timeout;
    let result = created(&request, &Settings::default());
    let BatchProviderCreateCall::Xai(call) = &result.call else {
        panic!("xai owns the branch without credentials");
    };
    assert_eq!(call.timeout, expected);
}

fn client_with_read(read: Option<f64>) -> Option<RequestTimeout> {
    Some(RequestTimeout::Client(HttpTimeout {
        connect: Some(1.0),
        read,
        write: Some(3.0),
        pool: Some(4.0),
    }))
}

#[rstest]
#[case::bedrock_without_model(BatchProvider::Bedrock)]
#[case::mistral_without_model(BatchProvider::Mistral)]
fn provider_create_error(#[case] provider: BatchProvider) {
    let (outcome, calls) = run(&input(provider), &Settings::default());
    let Err(error) = outcome else {
        panic!("an unconfigured provider is a provider error");
    };
    let status = error.status_code();
    let BatchProviderCreateError::UnsupportedProvider { provider: named } = error else {
        panic!("the error names the unsupported provider");
    };
    assert_eq!(named, provider);
    assert_eq!(status, Some(400));
    assert!(calls.is_empty());
}

#[rstest]
#[case::bedrock(BatchProvider::Bedrock)]
#[case::mistral(BatchProvider::Mistral)]
fn provider_config_routes_the_configured_providers(#[case] provider: BatchProvider) {
    let mut request = input(provider);
    request.model = Some(format!("{}/model", provider.as_str()));
    let result = created(&request, &Settings::default());
    assert!(matches!(
        result.call,
        BatchProviderCreateCall::ProviderConfig(_)
    ));
}

#[rstest]
#[case::openai(BatchProvider::Openai)]
#[case::hosted_vllm(BatchProvider::HostedVllm)]
#[case::litellm_proxy(BatchProvider::LitellmProxy)]
fn openai_compatible_providers_share_the_branch(#[case] provider: BatchProvider) {
    let result = created(&input(provider), &Settings::default());
    let BatchProviderCreateCall::OpenAiCompatible(call) = &result.call else {
        panic!("the openai-compatible set shares one branch");
    };
    assert_eq!(call.api_base.as_deref(), Some(DEFAULT_OPENAI_BASE));
}

#[rstest]
#[case::azure(BatchProvider::Azure)]
#[case::vertex(BatchProvider::VertexAi)]
#[case::xai(BatchProvider::Xai)]
fn other_providers_own_their_branch(#[case] provider: BatchProvider) {
    let result = created(&input(provider), &Settings::default());
    match (provider, &result.call) {
        (BatchProvider::Azure, BatchProviderCreateCall::Azure(_)) => {}
        (BatchProvider::VertexAi, BatchProviderCreateCall::VertexAi(_)) => {}
        (BatchProvider::Xai, BatchProviderCreateCall::Xai(_)) => {}
        (named, call) => panic!("{named} took the wrong branch: {call:?}"),
    }
}

type Probe = fn(&Settings, &BatchProviderCreateCall) -> Option<String>;

fn openai_base(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::OpenAiCompatible(call) => call.api_base.clone(),
        _other => None,
    }
}

fn openai_key(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::OpenAiCompatible(call) => revealed(call.api_key.as_ref()),
        _other => None,
    }
}

fn openai_organization(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::OpenAiCompatible(call) => call.organization.clone(),
        _other => None,
    }
}

fn azure_base(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::Azure(call) => call.api_base.clone(),
        _other => None,
    }
}

fn azure_version(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::Azure(call) => call.api_version.clone(),
        _other => None,
    }
}

fn azure_key(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::Azure(call) => revealed(call.api_key.as_ref()),
        _other => None,
    }
}

fn vertex_base(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::VertexAi(call) => call.api_base.clone(),
        _other => None,
    }
}

fn vertex_project(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::VertexAi(call) => call.vertex_project.clone(),
        _other => None,
    }
}

fn vertex_location(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::VertexAi(call) => call.vertex_location.clone(),
        _other => None,
    }
}

fn vertex_credentials(_settings: &Settings, call: &BatchProviderCreateCall) -> Option<String> {
    match call {
        BatchProviderCreateCall::VertexAi(call) => match &call.vertex_credentials {
            Some(VertexCredentials::Path(path)) => Some(path.clone()),
            Some(VertexCredentials::Fields(fields)) => fields.get("client_email").cloned(),
            None => None,
        },
        _other => None,
    }
}

fn settings_azure_ad_token(settings: &Settings, _call: &BatchProviderCreateCall) -> Option<String> {
    revealed(settings.azure_ad_token.as_ref())
}

fn load_settings(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .expect("the config loads")
        .sections
        .batch_provider_create
}

#[rstest]
#[case::litellm_settings_api_base("litellm_settings:\n  api_base: https://module.example/v1\n", &[], BatchProvider::Openai, openai_base, Some("https://module.example/v1"))]
#[case::environment_openai_base_url("", &[("OPENAI_BASE_URL", "https://base-url.example/v1")], BatchProvider::Openai, openai_base, Some("https://base-url.example/v1"))]
#[case::environment_openai_api_base("", &[("OPENAI_API_BASE", "https://api-base.example/v1")], BatchProvider::Openai, openai_base, Some("https://api-base.example/v1"))]
#[case::litellm_settings_organization("litellm_settings:\n  organization: module-org\n", &[], BatchProvider::Openai, openai_organization, Some("module-org"))]
#[case::environment_openai_organization("", &[("OPENAI_ORGANIZATION", "environment-org")], BatchProvider::Openai, openai_organization, Some("environment-org"))]
#[case::litellm_settings_api_key("litellm_settings:\n  api_key: module-key\n", &[], BatchProvider::Openai, openai_key, Some("module-key"))]
#[case::litellm_settings_openai_key("litellm_settings:\n  openai_key: openai-key\n", &[], BatchProvider::Openai, openai_key, Some("openai-key"))]
#[case::environment_openai_api_key("", &[("OPENAI_API_KEY", "openai-environment-key")], BatchProvider::Openai, openai_key, Some("openai-environment-key"))]
#[case::environment_azure_api_base("", &[("AZURE_API_BASE", "https://azure.example")], BatchProvider::Azure, azure_base, Some("https://azure.example"))]
#[case::litellm_settings_api_version("litellm_settings:\n  api_version: 2024-02-01\n", &[], BatchProvider::Azure, azure_version, Some("2024-02-01"))]
#[case::environment_azure_api_version("", &[("AZURE_API_VERSION", "2024-06-01")], BatchProvider::Azure, azure_version, Some("2024-06-01"))]
#[case::litellm_settings_azure_key("litellm_settings:\n  azure_key: azure-key\n", &[], BatchProvider::Azure, azure_key, Some("azure-key"))]
#[case::environment_azure_openai_api_key("", &[("AZURE_OPENAI_API_KEY", "azure-openai-environment-key")], BatchProvider::Azure, azure_key, Some("azure-openai-environment-key"))]
#[case::environment_azure_api_key("", &[("AZURE_API_KEY", "azure-environment-key")], BatchProvider::Azure, azure_key, Some("azure-environment-key"))]
#[case::environment_azure_ad_token("", &[("AZURE_AD_TOKEN", "azure-ad-token")], BatchProvider::Azure, settings_azure_ad_token, Some("azure-ad-token"))]
#[case::litellm_settings_vertex_project("litellm_settings:\n  vertex_project: module-project\n", &[], BatchProvider::VertexAi, vertex_project, Some("module-project"))]
#[case::environment_vertexai_project("", &[("VERTEXAI_PROJECT", "environment-project")], BatchProvider::VertexAi, vertex_project, Some("environment-project"))]
#[case::litellm_settings_vertex_location("litellm_settings:\n  vertex_location: module-location\n", &[], BatchProvider::VertexAi, vertex_location, Some("module-location"))]
#[case::environment_vertexai_location("", &[("VERTEXAI_LOCATION", "environment-location")], BatchProvider::VertexAi, vertex_location, Some("environment-location"))]
#[case::environment_vertexai_credentials("", &[("VERTEXAI_CREDENTIALS", "environment-credentials")], BatchProvider::VertexAi, vertex_credentials, Some("environment-credentials"))]
#[case::explicit_module_value_overrides_the_environment("litellm_settings:\n  api_base: https://module.example/v1\n", &[("OPENAI_BASE_URL", "https://base-url.example/v1")], BatchProvider::Openai, openai_base, Some("https://module.example/v1"))]
#[case::unset_defaults("", &[], BatchProvider::Openai, openai_base, Some("https://api.openai.com/v1"))]
fn loads_settings(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] provider: BatchProvider,
    #[case] probe: Probe,
    #[case] expected: Option<&str>,
) {
    let settings = load_settings(source, environment);
    let result = created(&input(provider), &settings);
    assert_eq!(probe(&settings, &result.call).as_deref(), expected);
}

#[derive(Clone, Copy, Debug)]
enum InvalidCase {
    MissingEnvironmentReference,
    WrongTypeValue,
    EmptyReferenceValue,
}

#[rstest]
#[case::missing_environment_reference(InvalidCase::MissingEnvironmentReference)]
#[case::wrong_type_value(InvalidCase::WrongTypeValue)]
#[case::empty_reference_value(InvalidCase::EmptyReferenceValue)]
fn loads_settings_handles_invalid_values(#[case] case: InvalidCase) {
    match case {
        InvalidCase::MissingEnvironmentReference => {
            let outcome = ConfigLoader {
                lookup: BTreeMap::<String, String>::new(),
            }
            .load("litellm_settings:\n  api_base: os.environ/MODULE_BASE\n");
            let Err(ConfigError::MissingEnvironment { variable, path }) = outcome else {
                panic!("a missing reference must name the variable and the setting");
            };
            assert_eq!(variable, "MODULE_BASE");
            assert_eq!(path, "litellm_settings.api_base");
        }
        InvalidCase::WrongTypeValue => {
            let outcome = ConfigLoader {
                lookup: BTreeMap::<String, String>::new(),
            }
            .load("litellm_settings:\n  organization: [one, two]\n");
            assert!(matches!(outcome, Err(ConfigError::Yaml { .. })));
        }
        InvalidCase::EmptyReferenceValue => {
            let settings = load_settings(
                "litellm_settings:\n  api_key: os.environ/MODULE_KEY\n",
                &[("MODULE_KEY", "")],
            );
            let result = created(&input(BatchProvider::Openai), &settings);
            assert_eq!(openai_key(&settings, &result.call), None);
        }
    }
}

fn settings_with(module: &[(&str, &str)], environment: &[(&str, &str)]) -> Settings {
    let mut settings = Settings::default();
    for (key, value) in module {
        match *key {
            "api_base" => settings.api_base = Some((*value).to_owned()),
            "organization" => settings.organization = Some((*value).to_owned()),
            "api_key" => settings.api_key = Some(SecretString::from(*value)),
            "openai_key" => settings.openai_key = Some(SecretString::from(*value)),
            "api_version" => settings.api_version = Some((*value).to_owned()),
            "azure_key" => settings.azure_key = Some(SecretString::from(*value)),
            "vertex_project" => settings.vertex_project = Some((*value).to_owned()),
            "vertex_location" => settings.vertex_location = Some((*value).to_owned()),
            _other => panic!("unknown module key {key}"),
        }
    }
    for (key, value) in environment {
        match *key {
            "OPENAI_BASE_URL" => settings.openai_base_url = Some((*value).to_owned()),
            "OPENAI_API_BASE" => settings.openai_api_base = Some((*value).to_owned()),
            "OPENAI_ORGANIZATION" => settings.openai_organization = Some((*value).to_owned()),
            "OPENAI_API_KEY" => settings.openai_api_key = Some(SecretString::from(*value)),
            "AZURE_API_BASE" => settings.azure_api_base = Some((*value).to_owned()),
            "AZURE_API_VERSION" => settings.azure_api_version = Some((*value).to_owned()),
            "AZURE_OPENAI_API_KEY" => {
                settings.azure_openai_api_key = Some(SecretString::from(*value));
            }
            "AZURE_API_KEY" => settings.azure_api_key = Some(SecretString::from(*value)),
            "AZURE_AD_TOKEN" => settings.azure_ad_token = Some(SecretString::from(*value)),
            "VERTEXAI_PROJECT" => settings.vertexai_project = Some((*value).to_owned()),
            "VERTEXAI_LOCATION" => settings.vertexai_location = Some((*value).to_owned()),
            "VERTEXAI_CREDENTIALS" => {
                settings.vertexai_credentials = Some(SecretString::from(*value));
            }
            _other => panic!("unknown environment key {key}"),
        }
    }
    settings
}

fn optional_with(entries: &[(&str, &str)]) -> BatchOptionalParams {
    let mut optional = BatchOptionalParams::default();
    for (key, value) in entries {
        match *key {
            "api_base" => optional.api_base = Some((*value).to_owned()),
            "api_key" => optional.api_key = Some(SecretString::from(*value)),
            "api_version" => optional.api_version = Some((*value).to_owned()),
            "organization" => optional.organization = Some((*value).to_owned()),
            "vertex_project" => optional.vertex_project = Some((*value).to_owned()),
            "vertex_location" => optional.vertex_location = Some((*value).to_owned()),
            "vertex_credentials" => {
                optional.vertex_credentials = Some(VertexCredentials::Path((*value).to_owned()));
            }
            _other => panic!("unknown optional key {key}"),
        }
    }
    optional
}

#[rstest]
#[case::openai_base_optional_wins(BatchProvider::Openai, &[("api_base", "module")], &[("OPENAI_BASE_URL", "environment")], &[("api_base", "optional")], openai_base, Some("optional"))]
#[case::openai_base_module_wins(BatchProvider::Openai, &[("api_base", "module")], &[("OPENAI_BASE_URL", "base-url"), ("OPENAI_API_BASE", "api-base")], &[], openai_base, Some("module"))]
#[case::openai_base_environment_order(BatchProvider::Openai, &[], &[("OPENAI_BASE_URL", "base-url"), ("OPENAI_API_BASE", "api-base")], &[], openai_base, Some("base-url"))]
#[case::openai_base_default(BatchProvider::Openai, &[], &[], &[], openai_base, Some(DEFAULT_OPENAI_BASE))]
#[case::openai_key_optional_wins(BatchProvider::Openai, &[("api_key", "module")], &[], &[("api_key", "optional")], openai_key, Some("optional"))]
#[case::openai_key_module_order(BatchProvider::Openai, &[("api_key", "module"), ("openai_key", "openai"), ("OPENAI_API_KEY", "environment")], &[], &[], openai_key, Some("module"))]
#[case::openai_key_openai_key_wins(BatchProvider::Openai, &[("openai_key", "openai")], &[("OPENAI_API_KEY", "environment")], &[], openai_key, Some("openai"))]
#[case::openai_key_environment(BatchProvider::Openai, &[], &[("OPENAI_API_KEY", "environment")], &[], openai_key, Some("environment"))]
#[case::organization_optional_wins(BatchProvider::Openai, &[("organization", "module")], &[("OPENAI_ORGANIZATION", "environment")], &[("organization", "optional")], openai_organization, Some("optional"))]
#[case::organization_module_wins(BatchProvider::Openai, &[("organization", "module")], &[("OPENAI_ORGANIZATION", "environment")], &[], openai_organization, Some("module"))]
#[case::azure_base_optional_wins(BatchProvider::Azure, &[("api_base", "module")], &[("AZURE_API_BASE", "environment")], &[("api_base", "optional")], azure_base, Some("optional"))]
#[case::azure_base_module_wins(BatchProvider::Azure, &[("api_base", "module")], &[("AZURE_API_BASE", "environment")], &[], azure_base, Some("module"))]
#[case::azure_version_optional_wins(BatchProvider::Azure, &[("api_version", "module")], &[("AZURE_API_VERSION", "environment")], &[("api_version", "optional")], azure_version, Some("optional"))]
#[case::azure_version_module_wins(BatchProvider::Azure, &[("api_version", "module")], &[("AZURE_API_VERSION", "environment")], &[], azure_version, Some("module"))]
#[case::azure_key_module_order(BatchProvider::Azure, &[("api_key", "module"), ("azure_key", "azure")], &[("AZURE_OPENAI_API_KEY", "openai-environment")], &[], azure_key, Some("module"))]
#[case::azure_key_azure_key_wins(BatchProvider::Azure, &[("azure_key", "azure")], &[("AZURE_OPENAI_API_KEY", "openai-environment")], &[], azure_key, Some("azure"))]
#[case::azure_key_environment_order(BatchProvider::Azure, &[], &[("AZURE_OPENAI_API_KEY", "openai-environment"), ("AZURE_API_KEY", "environment")], &[], azure_key, Some("openai-environment"))]
#[case::vertex_base_optional(BatchProvider::VertexAi, &[], &[], &[("api_base", "https://vertex.example")], vertex_base, Some("https://vertex.example"))]
#[case::vertex_base_defaults_empty(BatchProvider::VertexAi, &[], &[], &[], vertex_base, Some(""))]
#[case::vertex_project_optional_wins(BatchProvider::VertexAi, &[("vertex_project", "module")], &[("VERTEXAI_PROJECT", "environment")], &[("vertex_project", "optional")], vertex_project, Some("optional"))]
#[case::vertex_project_module_wins(BatchProvider::VertexAi, &[("vertex_project", "module")], &[("VERTEXAI_PROJECT", "environment")], &[], vertex_project, Some("module"))]
#[case::vertex_location_module_wins(BatchProvider::VertexAi, &[("vertex_location", "module")], &[("VERTEXAI_LOCATION", "environment")], &[], vertex_location, Some("module"))]
#[case::vertex_credentials_optional_wins(BatchProvider::VertexAi, &[], &[("VERTEXAI_CREDENTIALS", "environment")], &[("vertex_credentials", "optional")], vertex_credentials, Some("optional"))]
#[case::vertex_credentials_environment(BatchProvider::VertexAi, &[], &[("VERTEXAI_CREDENTIALS", "environment")], &[], vertex_credentials, Some("environment"))]
fn provider_credential_precedence(
    #[case] provider: BatchProvider,
    #[case] module: &[(&str, &str)],
    #[case] environment: &[(&str, &str)],
    #[case] optional: &[(&str, &str)],
    #[case] probe: Probe,
    #[case] expected: Option<&str>,
) {
    let settings = settings_with(module, environment);
    let mut request = input(provider);
    request.optional_params = optional_with(optional);
    let result = created(&request, &settings);
    assert_eq!(probe(&settings, &result.call).as_deref(), expected);
}

#[test]
fn xai_takes_only_the_request_credentials() {
    let settings = settings_with(&[("api_base", "module"), ("api_key", "module")], &[]);
    let result = created(&input(BatchProvider::Xai), &settings);
    let BatchProviderCreateCall::Xai(call) = &result.call else {
        panic!("xai owns its own branch");
    };
    assert_eq!(call.api_base, None);
    assert_eq!(revealed(call.api_key.as_ref()), None);
    let mut request = input(BatchProvider::Xai);
    request.optional_params = optional_with(&[("api_base", "optional"), ("api_key", "key")]);
    let result = created(&request, &settings);
    let BatchProviderCreateCall::Xai(call) = &result.call else {
        panic!("xai owns its own branch");
    };
    assert_eq!(call.api_base.as_deref(), Some("optional"));
    assert_eq!(revealed(call.api_key.as_ref()).as_deref(), Some("key"));
}

#[rstest]
#[case::openai(BatchProvider::Openai, "openai")]
#[case::azure(BatchProvider::Azure, "azure")]
#[case::vertex_ai(BatchProvider::VertexAi, "vertex_ai")]
#[case::bedrock(BatchProvider::Bedrock, "bedrock")]
#[case::hosted_vllm(BatchProvider::HostedVllm, "hosted_vllm")]
#[case::litellm_proxy(BatchProvider::LitellmProxy, "litellm_proxy")]
#[case::mistral(BatchProvider::Mistral, "mistral")]
#[case::xai(BatchProvider::Xai, "xai")]
fn provider_names_are_the_wire_slugs(#[case] provider: BatchProvider, #[case] expected: &str) {
    assert_eq!(provider.as_str(), expected);
}

#[test]
fn a_model_does_not_route_other_providers_to_a_provider_config() {
    let mut request = input(BatchProvider::Openai);
    request.model = Some("openai/gpt-4o".to_owned());
    let result = created(&request, &Settings::default());
    assert!(matches!(
        result.call,
        BatchProviderCreateCall::OpenAiCompatible(_)
    ));
}

#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::credentials::source::{CallableTokenProvider, ProviderFailure, TokenReply};
use betterlite_config::sections::image_generation_call::Settings;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_context::non_default_params::Params as MapperParams;
use betterlite_context::non_default_params::error::MapperError;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_error::dispatch::RawException;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse, ImageUsage};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use super::call::{CustomClientKind, GenericParams, ImageGenerationCall, LiteLlmCredentials};
use super::error::{ImageGenerationException, provider_failure};
use super::operations::{
    AzureRequest, BedrockRequest, BlackForestLabsRequest, Branch, BranchAnswer, CustomImageHandler,
    CustomProviderSlot, CustomRequest, EntraRequest, FoundryAuthRequest, FoundryRequest,
    GenericRequest, Headers, ImageGenerationOperations, OpenAiRequest, Params,
};
use super::plan::{
    ImageGenerationAnswer, ImageGenerationPlan, ImageIdentity, ImageResolutionError,
    ImageResolutionRequest, ResolvedImageModel, image_generation,
};

const NOW: i64 = 7;

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

fn logging_state() -> LoggingState {
    let moment =
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture is RFC 3339");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: String::from("image_generation"),
            start_time: moment,
            litellm_call_id: String::from("call-1"),
            function_id: String::from("fn-1"),
            messages: None,
            stream: None,
            litellm_trace_id: Some(String::from("trace-1")),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("generated-trace"),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

fn object(pairs: &[(&str, &str)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
            .collect(),
    )
}

fn kwargs(pairs: Vec<(&str, Json)>) -> IndexMap<String, Json> {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn call(model: Option<&str>, provider: Option<&str>) -> ImageGenerationCall {
    ImageGenerationCall {
        model: model.map(str::to_owned),
        prompt: String::from("cat"),
        n: None,
        quality: None,
        response_format: None,
        size: None,
        style: None,
        user: None,
        timeout: 600.0,
        api_key: None,
        api_base: None,
        api_version: None,
        custom_llm_provider: provider.map(str::to_owned),
        client_kind: None,
        now_seconds: NOW,
        kwargs: IndexMap::new(),
    }
}

#[derive(Debug, Clone)]
struct SeenResolution {
    model: Option<String>,
    provider: Option<String>,
    api_base: Option<String>,
    has_key: bool,
}

struct FakeIdentity {
    result: Result<ResolvedImageModel, ImageResolutionError>,
    seen: Mutex<Vec<SeenResolution>>,
}

impl FakeIdentity {
    fn new(model: Option<&str>, provider: ProviderIdentity) -> Self {
        let resolved = ResolvedImageModel {
            model: model.map(str::to_owned),
            provider,
            api_base: None,
            api_key: None,
        };
        let seen = Mutex::new(Vec::new());
        Self {
            result: Ok(resolved),
            seen,
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            result: Err(ImageResolutionError::Rejected {
                message: message.to_owned(),
            }),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<SeenResolution> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageIdentity for FakeIdentity {
    fn resolve(
        &self,
        request: &ImageResolutionRequest<'_>,
    ) -> Result<ResolvedImageModel, ImageResolutionError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenResolution {
                model: request.model.map(str::to_owned),
                provider: request.custom_llm_provider.map(str::to_owned),
                api_base: request.api_base.map(str::to_owned),
                has_key: request.api_key.is_some(),
            });
        self.result.clone()
    }
}

struct TestMapper {
    supported: Vec<String>,
}

impl OptionalParamsMapper for TestMapper {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn map_openai_params(
        &self,
        non_default_params: &MapperParams,
        mut optional_params: MapperParams,
        _model: &str,
        _drop_params: bool,
    ) -> Result<MapperParams, MapperError> {
        optional_params.extend(non_default_params.clone());
        Ok(optional_params)
    }
}

struct StaticProvider;

impl CallableTokenProvider for StaticProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(SecretString::from("entra-token")))
    }
}

#[derive(Debug, Clone)]
struct Sent {
    branch: &'static str,
    model: Option<String>,
    prompt: String,
    timeout: Duration,
    api_key: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    azure_ad_token: Option<String>,
    has_entra_provider: bool,
    options: Vec<(String, Json)>,
    headers: Vec<(String, String)>,
    litellm_params: Vec<(String, Json)>,
    base_model: Option<String>,
    organization: Option<String>,
    asynchronous: bool,
    now_seconds: i64,
    client: Option<CustomClientKind>,
    config: Option<ImageGenerationConfigTag>,
    provider: Option<String>,
}

impl Sent {
    fn option(&self, name: &str) -> Option<&Json> {
        self.options
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }

    fn litellm(&self, name: &str) -> Option<&Json> {
        self.litellm_params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value))
    }
}

fn pairs<'item, Value, Items>(items: Items) -> Vec<(String, Value)>
where
    Items: IntoIterator<Item = (&'item String, &'item Value)>,
    Value: Clone + 'item,
{
    items
        .into_iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// what the fake reads off each branch request; a field the branch does not carry stays empty
trait Recorded {
    fn model(&self) -> Option<&str>;
    fn prompt(&self) -> &str;
    fn timeout(&self) -> Duration;
    fn options(&self) -> &Params;
    fn api_key(&self) -> Option<&SecretString> {
        None
    }
    fn api_base(&self) -> Option<String> {
        None
    }
    fn api_version(&self) -> Option<&str> {
        None
    }
    fn azure_ad_token(&self) -> Option<&SecretString> {
        None
    }
    fn has_entra_provider(&self) -> bool {
        false
    }
    fn headers(&self) -> Option<&Headers> {
        None
    }
    fn litellm_params(&self) -> Option<&Params> {
        None
    }
    fn base_model(&self) -> Option<&str> {
        None
    }
    fn organization(&self) -> Option<&str> {
        None
    }
    fn asynchronous(&self) -> bool {
        false
    }
    fn now_seconds(&self) -> i64 {
        0_i64
    }
    fn client(&self) -> Option<CustomClientKind> {
        None
    }
    fn config(&self) -> Option<ImageGenerationConfigTag> {
        None
    }
    fn provider(&self) -> Option<&str> {
        None
    }
}

impl Recorded for AzureRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.map(str::to_owned)
    }
    fn api_version(&self) -> Option<&str> {
        self.api_version
    }
    fn azure_ad_token(&self) -> Option<&SecretString> {
        self.azure_ad_token
    }
    fn has_entra_provider(&self) -> bool {
        self.azure_ad_token_provider.is_some()
    }
    fn headers(&self) -> Option<&Headers> {
        Some(self.headers)
    }
    fn litellm_params(&self) -> Option<&Params> {
        Some(self.litellm_params)
    }
    fn base_model(&self) -> Option<&str> {
        self.base_model
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
    fn now_seconds(&self) -> i64 {
        self.now_seconds
    }
}

impl Recorded for GenericRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        match self.litellm_params.get("api_base") {
            Some(Json::Text(text)) => Some(text.clone()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        }
    }
    fn litellm_params(&self) -> Option<&Params> {
        Some(self.litellm_params)
    }
    fn config(&self) -> Option<ImageGenerationConfigTag> {
        Some(self.config.clone())
    }
    fn provider(&self) -> Option<&str> {
        Some(self.provider)
    }
}

impl Recorded for BlackForestLabsRequest<'_> {
    fn model(&self) -> Option<&str> {
        Some(self.model)
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn litellm_params(&self) -> Option<&Params> {
        Some(self.litellm_params)
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
}

impl Recorded for FoundryRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.map(str::to_owned)
    }
    fn api_version(&self) -> Option<&str> {
        self.api_version
    }
    fn azure_ad_token(&self) -> Option<&SecretString> {
        self.azure_ad_token
    }
    fn has_entra_provider(&self) -> bool {
        self.azure_ad_token_provider.is_some()
    }
    fn headers(&self) -> Option<&Headers> {
        Some(self.headers)
    }
    fn litellm_params(&self) -> Option<&Params> {
        Some(self.litellm_params)
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
}

impl Recorded for OpenAiRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.map(str::to_owned)
    }
    fn headers(&self) -> Option<&Headers> {
        Some(self.headers)
    }
    fn organization(&self) -> Option<&str> {
        self.organization
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
    fn now_seconds(&self) -> i64 {
        self.now_seconds
    }
}

impl Recorded for BedrockRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.map(str::to_owned)
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
}

impl Recorded for CustomRequest<'_> {
    fn model(&self) -> Option<&str> {
        self.model
    }
    fn prompt(&self) -> &str {
        self.prompt
    }
    fn timeout(&self) -> Duration {
        self.timeout
    }
    fn options(&self) -> &Params {
        self.optional_params
    }
    fn api_key(&self) -> Option<&SecretString> {
        self.api_key
    }
    fn api_base(&self) -> Option<String> {
        self.api_base.map(str::to_owned)
    }
    fn asynchronous(&self) -> bool {
        self.asynchronous
    }
    fn client(&self) -> Option<CustomClientKind> {
        self.client
    }
    fn provider(&self) -> Option<&str> {
        Some(self.provider)
    }
}

fn snapshot(branch: &Branch<'_>) -> Sent {
    match branch {
        Branch::Azure(request) => record("azure", request),
        Branch::Generic(request) => record("generic", request),
        Branch::BlackForestLabs(request) => record("black_forest_labs", request),
        Branch::Foundry(request) => record("azure_ai", request),
        Branch::OpenAi(request) => record("openai", request),
        Branch::Bedrock(request) => record("bedrock", request),
        Branch::Custom(request) => record("custom", request),
    }
}

fn record(branch: &'static str, request: &dyn Recorded) -> Sent {
    Sent {
        branch,
        model: request.model().map(str::to_owned),
        prompt: request.prompt().to_owned(),
        timeout: request.timeout(),
        api_key: request
            .api_key()
            .map(|secret| secret.expose_secret().to_owned()),
        api_base: request.api_base(),
        api_version: request.api_version().map(str::to_owned),
        azure_ad_token: request
            .azure_ad_token()
            .map(|secret| secret.expose_secret().to_owned()),
        has_entra_provider: request.has_entra_provider(),
        options: pairs(request.options()),
        headers: request.headers().map(pairs).unwrap_or_default(),
        litellm_params: request.litellm_params().map(pairs).unwrap_or_default(),
        base_model: request.base_model().map(str::to_owned),
        organization: request.organization().map(str::to_owned),
        asynchronous: request.asynchronous(),
        now_seconds: request.now_seconds(),
        client: request.client(),
        config: request.config(),
        provider: request.provider().map(str::to_owned),
    }
}

struct FakeOps {
    config: Option<ImageGenerationConfigTag>,
    supported: Vec<String>,
    configs_seen: Mutex<Vec<(Option<String>, String)>>,
    entra_seen: Mutex<Vec<(String, String, String, String)>>,
    foundry_seen: Mutex<Vec<(Option<String>, Vec<(String, Json)>)>>,
    sends: Mutex<Vec<Sent>>,
    answer: BranchAnswer,
    send_failure: Option<(u16, String)>,
}

impl FakeOps {
    fn new(config: Option<ImageGenerationConfigTag>) -> Self {
        Self {
            config,
            supported: vec![
                String::from("n"),
                String::from("quality"),
                String::from("response_format"),
                String::from("size"),
                String::from("style"),
                String::from("user"),
                String::from("azure_ad_token"),
            ],
            configs_seen: Mutex::new(Vec::new()),
            entra_seen: Mutex::new(Vec::new()),
            foundry_seen: Mutex::new(Vec::new()),
            sends: Mutex::new(Vec::new()),
            answer: BranchAnswer {
                response: ImageResponse {
                    created: NOW,
                    data: Some(vec![ImageData {
                        b64_json: None,
                        revised_prompt: None,
                        url: Some(String::from("https://example.test/generated.png")),
                    }]),
                    usage: Some(ImageUsage::default()),
                },
                hidden_model: None,
            },
            send_failure: None,
        }
    }

    fn failing(config: Option<ImageGenerationConfigTag>, status: u16, message: &str) -> Self {
        let mut ops = Self::new(config);
        ops.send_failure = Some((status, message.to_owned()));
        ops
    }

    fn sends(&self) -> Vec<Sent> {
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn configs(&self) -> Vec<(Option<String>, String)> {
        self.configs_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn entra(&self) -> Vec<(String, String, String, String)> {
        self.entra_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn foundry(&self) -> Vec<(Option<String>, Vec<(String, Json)>)> {
        self.foundry_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ImageGenerationOperations for FakeOps {
    fn generation_config(
        &self,
        model: Option<&str>,
        provider: &ProviderIdentity,
    ) -> Option<ImageGenerationConfigTag> {
        let seen = (model.map(str::to_owned), provider.slug().to_owned());
        self.configs_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        self.config.clone()
    }

    fn options_mapper(
        &self,
        _config: &ImageGenerationConfigTag,
    ) -> Option<Box<dyn OptionalParamsMapper + '_>> {
        Some(Box::new(TestMapper {
            supported: self.supported.clone(),
        }))
    }

    fn bedrock_mapper(&self) -> Option<&dyn OptionalParamsMapper> {
        None
    }

    fn entra_token_provider(
        &self,
        request: EntraRequest<'_>,
    ) -> Option<Arc<dyn CallableTokenProvider>> {
        self.entra_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request.tenant_id.to_owned(),
                request.client_id.to_owned(),
                request.client_secret.to_owned(),
                request.scope.to_owned(),
            ));
        Some(Arc::new(StaticProvider))
    }

    fn foundry_auth_headers(
        &self,
        request: FoundryAuthRequest<'_>,
    ) -> Result<Headers, Box<RawException>> {
        self.foundry_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request
                    .api_key
                    .map(|secret| secret.expose_secret().to_owned()),
                pairs(request.litellm_params),
            ));
        let mut headers = Headers::new();
        if let Some(key) = request.api_key {
            headers.insert(String::from("api-key"), key.expose_secret().to_owned());
        }
        Ok(headers)
    }

    fn send(
        &self,
        branch: Branch<'_>,
        _logging: &mut LoggingState,
    ) -> impl Future<Output = Result<BranchAnswer, Box<RawException>>> + Send {
        let sent = snapshot(&branch);
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
        let failure = self.send_failure.clone();
        let answer = self.answer.clone();
        async move {
            match failure {
                Some((status, message)) => Err(Box::new(provider_failure(
                    status,
                    &message,
                    "image-model",
                    "custom",
                    "RateLimitError",
                ))),
                None => Ok(answer),
            }
        }
    }
}

struct Case<'case> {
    ops: &'case FakeOps,
    identity: &'case FakeIdentity,
    settings: Settings,
    environment: BTreeMap<String, String>,
    custom_providers: BTreeSet<String>,
    custom_provider_map: Vec<CustomProviderSlot>,
    azure_ad_token_provider: Option<Arc<dyn CallableTokenProvider>>,
    drop_params: bool,
    runtime: Runtime,
}

impl<'case> Case<'case> {
    fn new(ops: &'case FakeOps, identity: &'case FakeIdentity, settings: Settings) -> Self {
        Self {
            ops,
            identity,
            settings,
            environment: BTreeMap::new(),
            custom_providers: BTreeSet::new(),
            custom_provider_map: Vec::new(),
            azure_ad_token_provider: None,
            drop_params: false,
            runtime: Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the test runtime builds"),
        }
    }

    fn run(
        &self,
        call: ImageGenerationCall,
    ) -> (
        Result<ImageGenerationAnswer, ImageGenerationException>,
        LoggingState,
    ) {
        let redactor = redactor();
        let plan = ImageGenerationPlan {
            identity: self.identity,
            operations: self.ops,
            settings: &self.settings,
            environment: &self.environment,
            custom_providers: &self.custom_providers,
            custom_provider_map: &self.custom_provider_map,
            azure_ad_token_provider: self.azure_ad_token_provider.clone(),
            drop_params: self.drop_params,
            redactor: &redactor,
        };
        let mut logging = logging_state();
        let answer = self
            .runtime
            .block_on(image_generation(&plan, call, &mut logging));
        (answer, logging)
    }
}

fn azure_identity(model: &str) -> FakeIdentity {
    FakeIdentity::new(Some(model), ProviderIdentity::Known(ProviderId::Azure))
}

mod cases;
mod credentials;

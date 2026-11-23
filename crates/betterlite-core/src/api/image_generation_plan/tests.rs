#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::image_generation_call::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_context::media_config_selection::image_generation::{
    ImageGenerationConfigTag, select_image_generation,
};
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_error::dispatch::RawException;
use betterlite_error::envelope::WireField;
use betterlite_error::error::{ErrorClass, ErrorContext, class_message};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::otel::proxy_runtime::slot::{RequestSlot, request_slot_scope};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ProviderIdentity, ResolutionContext};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageInputTokens, ImageResponse, ImageUsage,
};
use betterlite_openai::image_generation::call::ImageGenerationCall as OpenAiCall;
use betterlite_openai::image_generation::error::ImageGenerationFailure;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_openai::image_generation::request::{
    begin_image_generation, finish_image_generation, send_image_generation,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;
use tokio::runtime::{Builder, Runtime};

use super::error::{AimageGenerationError, refused};
use super::invocation::{GenerationReturn, Invocation, ModelPosition, PendingGeneration};
use super::{AimageGenerationPlan, ImageGenerationEntry, PlanEntry, aimage_generation};
use crate::api::image_generation_call::call::ImageGenerationCall;
use crate::api::image_generation_call::error::{
    ImageGenerationException, provider_failure, unknown_provider, unsupported_params, value_failure,
};
use crate::api::image_generation_call::operations::{
    Branch, BranchAnswer, EntraRequest, FoundryAuthRequest, Headers, ImageGenerationOperations,
    OpenAiRequest,
};
use crate::api::image_generation_call::plan::{CatalogIdentity, ImageGenerationPlan};

const NOW: i64 = 7;
const ANSWER: &str = r#"{"created":1,"data":[{"b64_json":"YQ=="}]}"#;
const INVALID_RESPONSE: &str = "Unable to get Image Response. Please pass a valid llm_provider.";

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
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

fn call_of(model: Option<&str>, prompt: &str) -> ImageGenerationCall {
    ImageGenerationCall {
        model: model.map(str::to_owned),
        prompt: prompt.to_owned(),
        image_count: None,
        quality: None,
        response_format: None,
        size: None,
        style: None,
        user: None,
        timeout: 600.0,
        api_key: None,
        api_base: None,
        api_version: None,
        custom_llm_provider: None,
        client_kind: None,
        now_seconds: NOW,
        kwargs: IndexMap::new(),
    }
}

fn keyword_call(model: Option<&str>) -> Invocation {
    Invocation {
        call: call_of(model, "cat"),
        model_position: ModelPosition::Keyword,
    }
}

// the caller wrote model=None, so the key is there and the wrapper passes the null on
fn null_keyword_call() -> Invocation {
    Invocation {
        call: call_of(None, "cat"),
        model_position: ModelPosition::KeywordNull,
    }
}

fn positional_call(prompt: &str, model: Option<&str>) -> Invocation {
    Invocation {
        call: call_of(model, prompt),
        model_position: ModelPosition::Positional,
    }
}

// the ImageResponse the three accepted return shapes all produce
fn empty_created_and_data() -> ImageResponse {
    ImageResponse {
        created: 1_i64,
        data: Some(Vec::new()),
        usage: None,
    }
}

fn answered_by_the_mock() -> ImageResponse {
    ImageResponse {
        created: 1_i64,
        data: Some(vec![ImageData {
            b64_json: Some(String::from("YQ==")),
            revised_prompt: None,
            url: None,
        }]),
        usage: Some(ImageUsage::default()),
    }
}

// a dict return with the two fields the DoD names
fn created_and_empty_data() -> Json {
    Json::Object(IndexMap::from([
        (String::from("created"), Json::Number(Number::from(1_i64))),
        (String::from("data"), Json::List(Vec::new())),
    ]))
}

fn connection(class: &ErrorClass) -> &ErrorContext {
    let ErrorClass::ApiConnection(context) = class else {
        panic!("the failure maps to APIConnectionError, got {class:?}");
    };
    context
}

fn raised_class(error: &AimageGenerationError) -> (&ErrorClass, &str) {
    let AimageGenerationError::Raised { class, message } = error else {
        panic!("the wrapper raises a mapped exception, got {error:?}");
    };
    (class, message)
}

// B011's openai.APIError fields: status 500, and type/param/code that are null rather than absent
fn assert_sdk_failure_fields(class: &ErrorClass) {
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500, "B011's SDK status");
    assert_eq!(metadata.error_type, WireField::Null, "type is nullable");
    assert_eq!(metadata.param, WireField::Null, "param is nullable");
    assert_eq!(metadata.openai_code, None, "code is nullable");
}

// the resolution world the wrapper's get_llm_provider reads, with no model list and no environment
struct Fixtures {
    catalog: Arc<ModelCatalog>,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
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

// What the entry returns for one call; Unexpected stands in for a return that is none of the three shapes.
enum Script {
    Response(ImageResponse),
    Mapping(Json),
    Coroutine(
        Result<ImageResponse, ImageGenerationException>,
        Arc<AtomicUsize>,
    ),
    Unexpected,
    Failure(ImageGenerationException),
}

struct ScriptedEntry {
    script: Mutex<Option<Script>>,
    calls: AtomicUsize,
    seen: Mutex<Vec<Invocation>>,
}

impl ScriptedEntry {
    fn new(script: Script) -> Self {
        Self {
            script: Mutex::new(Some(script)),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl ImageGenerationEntry for ScriptedEntry {
    fn generate<'call>(
        &'call self,
        invocation: &Invocation,
        _logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<GenerationReturn<'call>, ImageGenerationException>> + Send + 'call
    {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(invocation.clone());
        let script = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        scripted_return(script)
    }
}

fn scripted_return<'call>(
    script: Option<Script>,
) -> impl Future<Output = Result<GenerationReturn<'call>, ImageGenerationException>> + Send + 'call
{
    ready(match script {
        None => panic!("the scripted entry runs once"),
        Some(Script::Response(response)) => Ok(GenerationReturn::Response(Box::new(response))),
        Some(Script::Mapping(mapping)) => Ok(GenerationReturn::Mapping(mapping)),
        Some(Script::Coroutine(answer, awaits)) => {
            let pending: PendingGeneration<'call> = Box::pin(async move {
                awaits.fetch_add(1, Ordering::SeqCst);
                answer
            });
            Ok(GenerationReturn::Coroutine(pending))
        }
        Some(Script::Unexpected) => Ok(GenerationReturn::Unexpected),
        Some(Script::Failure(failure)) => Err(failure),
    })
}

struct ScriptedCase {
    fixtures: Fixtures,
    redactor: Redactor,
    runtime: Runtime,
}

impl ScriptedCase {
    fn new() -> Self {
        Self {
            fixtures: Fixtures::new(),
            redactor: redactor(),
            runtime: runtime(),
        }
    }

    fn run(
        &self,
        entry: &ScriptedEntry,
        invocation: Invocation,
    ) -> Result<ImageResponse, AimageGenerationError> {
        let resolution = self.fixtures.context();
        let facade = AimageGenerationPlan {
            entry,
            resolution: &resolution,
            redactor: &self.redactor,
        };
        let mut logging = logging_state();
        self.runtime
            .block_on(aimage_generation(&facade, invocation, &mut logging))
    }
}

fn scripted(
    script: Script,
    invocation: Invocation,
) -> (Result<ImageResponse, AimageGenerationError>, ScriptedEntry) {
    let case = ScriptedCase::new();
    let entry = ScriptedEntry::new(script);
    let result = case.run(&entry, invocation);
    (result, entry)
}

fn returned_response_is_used_as_it_is() {
    let (result, entry) = scripted(
        Script::Response(empty_created_and_data()),
        keyword_call(Some("openai/dall-e-2")),
    );
    assert_eq!(
        result.expect("a returned ImageResponse is used as it is"),
        empty_created_and_data()
    );
    assert_eq!(
        entry.calls.load(Ordering::SeqCst),
        1,
        "an ImageResponse answer is not generated a second time"
    );
}

fn mapping_is_built_into_a_response() {
    let (result, entry) = scripted(
        Script::Mapping(created_and_empty_data()),
        keyword_call(Some("openai/dall-e-2")),
    );
    assert_eq!(
        result.expect("a dict is built into an ImageResponse"),
        empty_created_and_data()
    );
    assert_eq!(entry.calls.load(Ordering::SeqCst), 1);
}

fn coroutine_is_awaited_once() {
    let awaits = Arc::new(AtomicUsize::new(0));
    let (result, entry) = scripted(
        Script::Coroutine(Ok(empty_created_and_data()), Arc::clone(&awaits)),
        keyword_call(Some("openai/dall-e-2")),
    );
    assert_eq!(
        result.expect("the coroutine's answer is awaited"),
        empty_created_and_data()
    );
    assert_eq!(
        awaits.load(Ordering::SeqCst),
        1,
        "the coroutine is awaited once"
    );
    assert_eq!(entry.calls.load(Ordering::SeqCst), 1);
}

// the production entry is a coroutine, so its provider failure surfaces only when the wrapper awaits it
fn failing_coroutine_reaches_the_caller() {
    let awaits = Arc::new(AtomicUsize::new(0));
    let (result, entry) = scripted(
        Script::Coroutine(
            Err(unsupported_params("n is not supported")),
            Arc::clone(&awaits),
        ),
        keyword_call(Some("openai/dall-e-2")),
    );
    let error = result.expect_err("the coroutine's exception is re-raised");
    let (class, message) = raised_class(&error);
    assert!(
        matches!(class, ErrorClass::UnsupportedParams(_)),
        "the awaited failure keeps its class, got {class:?}"
    );
    assert_eq!(
        message,
        "litellm.UnsupportedParamsError: n is not supported"
    );
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(
        awaits.load(Ordering::SeqCst),
        1,
        "the failing coroutine is awaited once"
    );
    assert_eq!(entry.calls.load(Ordering::SeqCst), 1);
}

fn unexpected_return_is_refused() {
    let (result, entry) = scripted(Script::Unexpected, keyword_call(Some("openai/dall-e-2")));
    let error = result.expect_err("an integer return is refused");
    assert_eq!(error.to_string(), INVALID_RESPONSE);
    let (class, message) = raised_class(&error);
    assert_eq!(message, INVALID_RESPONSE);
    assert!(
        class_message(class).starts_with("APIConnectionError: OpenAIException - "),
        "the ValueError is mapped under the resolved openai provider: {}",
        class_message(class)
    );
    assert_eq!(connection(class).provider.as_deref(), Some("openai"));
    assert_sdk_failure_fields(class);
    assert_eq!(entry.calls.load(Ordering::SeqCst), 1);
}

fn unexpected_return_keeps_the_azure_provider() {
    let (result, _) = scripted(Script::Unexpected, keyword_call(Some("azure/dall-e-3")));
    let error = result.expect_err("an integer return is refused");
    let (class, _) = raised_class(&error);
    assert_eq!(
        connection(class).provider.as_deref(),
        Some("azure"),
        "resolved azure stays azure"
    );
    assert!(
        class_message(class).contains("AzureException"),
        "the azure exception names its provider: {}",
        class_message(class)
    );
}

fn caller_provider_does_not_drive_attribution() {
    let mut invocation = keyword_call(Some("openai/dall-e-2"));
    invocation.call.custom_llm_provider = Some(String::from("azure"));
    let (result, _) = scripted(Script::Unexpected, invocation);
    let error = result.expect_err("the integer return is refused");
    let (class, _) = raised_class(&error);
    assert_eq!(
        connection(class).provider.as_deref(),
        Some("openai"),
        "only the model and api_base reach get_llm_provider"
    );
}

fn empty_provider_falls_back_to_openai() {
    let fallback = refused(&redactor(), INVALID_RESPONSE, Some("some-model"), "");
    let (class, _) = raised_class(&fallback);
    assert_eq!(
        connection(class).provider.as_deref(),
        Some("openai"),
        "an empty provider falls back to openai"
    );
}

fn unresolved_model_is_a_bad_request() {
    let (result, entry) = scripted(
        Script::Response(empty_created_and_data()),
        keyword_call(Some("not-a-real-model")),
    );
    let error = result.expect_err("the provider cannot be resolved");
    let (class, message) = raised_class(&error);
    assert!(
        matches!(class, ErrorClass::BadRequest(_)),
        "get_llm_provider's own BadRequestError is returned, got {class:?}"
    );
    assert!(
        message.starts_with("LLM Provider NOT provided"),
        "the failure keeps get_llm_provider's message: {message}"
    );
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(
        entry.calls.load(Ordering::SeqCst),
        0,
        "no generation runs once attribution failed"
    );
}

fn callee_unknown_provider_reaches_the_caller() {
    let (result, _) = scripted(
        Script::Failure(unknown_provider(Some("dall-e-3"), "azure")),
        keyword_call(Some("azure/dall-e-3")),
    );
    let error = result.expect_err("the callee's exception is re-raised");
    let AimageGenerationError::UnknownProvider { message } = error else {
        panic!("LiteLLMUnknownProvider is carried as its own exception");
    };
    assert_eq!(
        message,
        unknown_provider(Some("dall-e-3"), "azure").message()
    );
}

fn callee_mapped_exception_reaches_the_caller() {
    let (result, _) = scripted(
        Script::Failure(unsupported_params("n is not supported")),
        keyword_call(Some("openai/dall-e-2")),
    );
    let error = result.expect_err("the callee's exception is re-raised");
    let (class, message) = raised_class(&error);
    assert!(matches!(class, ErrorClass::UnsupportedParams(_)));
    assert_eq!(
        message,
        "litellm.UnsupportedParamsError: n is not supported"
    );
}

fn missing_model_fails_before_any_call() {
    let mut invocation = keyword_call(None);
    invocation
        .call
        .kwargs
        .insert(String::from("n"), Json::Text(String::from("not-a-number")));
    let (result, entry) = scripted(Script::Response(empty_created_and_data()), invocation);
    let error = result.expect_err("the model is required");
    assert_eq!(error.to_string(), "'model'");
    assert!(matches!(error, AimageGenerationError::MissingModel));
    assert_eq!(
        entry.calls.load(Ordering::SeqCst),
        0,
        "no call is dispatched without a model"
    );
}

// get_llm_provider(model=None) raises its own BadRequestError, which is what a present null reaches
fn null_model_is_a_bad_request() {
    let (result, entry) = scripted(
        Script::Response(empty_created_and_data()),
        null_keyword_call(),
    );
    let error = result.expect_err("get_llm_provider rejects a null model");
    let (class, message) = raised_class(&error);
    assert!(
        matches!(class, ErrorClass::BadRequest(_)),
        "get_llm_provider's own BadRequestError is returned, got {class:?}"
    );
    assert_eq!(
        message,
        "GetLLMProvider Exception - model parameter is required but was None. Please provide a valid model name.\n\noriginal model: None"
    );
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(
        class.metadata().provider.as_deref(),
        Some(""),
        "get_llm_provider names no provider for a None model"
    );
    assert_eq!(
        entry.calls.load(Ordering::SeqCst),
        0,
        "no generation runs for a null model"
    );
}

fn positional_model_drives_attribution() {
    let (result, entry) = scripted(
        Script::Unexpected,
        positional_call("openai/dall-e-2", Some("azure/dall-e-3")),
    );
    let error = result.expect_err("the integer return is refused");
    let (class, _) = raised_class(&error);
    assert_eq!(
        connection(class).provider.as_deref(),
        Some("openai"),
        "the first positional argument is the model, not the keyword model"
    );
    let seen = entry
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model_position, ModelPosition::Positional);
    assert_eq!(
        seen[0].call.prompt, "openai/dall-e-2",
        "generation receives the positional argument as its prompt"
    );
    assert_eq!(
        seen[0].call.model.as_deref(),
        Some("azure/dall-e-3"),
        "the keyword model is still passed through untouched"
    );
}

fn positional_model_alone_resolves() {
    let (result, entry) = scripted(
        Script::Response(empty_created_and_data()),
        positional_call("openai/dall-e-2", None),
    );
    assert_eq!(
        result.expect("the positional model resolves"),
        empty_created_and_data()
    );
    assert_eq!(entry.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn async_generation_result_shapes() {
    returned_response_is_used_as_it_is();
    mapping_is_built_into_a_response();
    coroutine_is_awaited_once();
    failing_coroutine_reaches_the_caller();
    unexpected_return_is_refused();
    unexpected_return_keeps_the_azure_provider();
    caller_provider_does_not_drive_attribution();
    empty_provider_falls_back_to_openai();
    unresolved_model_is_a_bad_request();
    callee_unknown_provider_reaches_the_caller();
    callee_mapped_exception_reaches_the_caller();
}

#[test]
fn async_generation_model_lookup() {
    missing_model_fails_before_any_call();
    null_model_is_a_bad_request();
    positional_model_drives_attribution();
    positional_model_alone_resolves();
}

// what the fixture observed at the openai branch, including the request slot it ran under
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotCheck {
    Match,
    Miss,
}

#[derive(Debug, Clone)]
struct Seen {
    model: Option<String>,
    prompt: String,
    asynchronous: bool,
    has_key: bool,
    slot: SlotCheck,
}

// The provider work B998's plan asks for, with the openai branch going over a real socket to the fixture.
struct FixtureOps {
    source: ProxiedClient,
    sends: Mutex<Vec<Seen>>,
}

impl FixtureOps {
    fn new(source: ProxiedClient) -> Self {
        Self {
            source,
            sends: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &OpenAiRequest<'_>, slot: SlotCheck) {
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                model: request.model.map(str::to_owned),
                prompt: request.prompt.to_owned(),
                asynchronous: request.asynchronous,
                has_key: request.api_key.is_some(),
                slot,
            });
    }

    async fn openai(
        &self,
        request: OpenAiRequest<'_>,
        logging: &LoggingState,
    ) -> Result<BranchAnswer, Box<RawException>> {
        // the wrapper runs the entry inside the slot LoggingState captured, so a fresh scope answers that slot
        let (active, ()) = request_slot_scope(async {}).await;
        let slot = if logging
            .request_slot
            .as_ref()
            .is_some_and(|expected| Arc::ptr_eq(expected, &active))
        {
            SlotCheck::Match
        } else {
            SlotCheck::Miss
        };
        self.record(&request, slot);
        let model = request.model.unwrap_or_default();
        let call = OpenAiCall {
            model: model.to_owned(),
            prompt: request.prompt.to_owned(),
            optional_params: request.optional_params.clone(),
            api_key: request.api_key.cloned(),
            api_base: request.api_base.map(str::to_owned),
            organization: request.organization.map(str::to_owned),
            headers: Some(request.headers.clone()),
            timeout: request.timeout,
            delegate_async: request.asynchronous,
            now_seconds: request.now_seconds,
        };
        let mut logger = SilentLogger;
        let prepared = begin_image_generation(call, &mut logger)
            .map_err(|failure| failure_of(&failure, model))?;
        let sent = send_image_generation(&prepared, &self.source).await;
        let outcome = finish_image_generation(&prepared, sent, &mut logger)
            .map_err(|failure| failure_of(&failure, model))?;
        Ok(BranchAnswer {
            response: reply_response(&outcome.reply),
            hidden_model: None,
        })
    }
}

impl ImageGenerationOperations for FixtureOps {
    fn generation_config(
        &self,
        model: Option<&str>,
        provider: &ProviderIdentity,
    ) -> Option<ImageGenerationConfigTag> {
        match provider {
            ProviderIdentity::Known(id) => select_image_generation(model.unwrap_or_default(), *id),
            ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
        }
    }

    fn options_mapper(
        &self,
        _config: &ImageGenerationConfigTag,
    ) -> Option<Box<dyn OptionalParamsMapper + '_>> {
        None
    }

    fn bedrock_mapper(&self) -> Option<&dyn OptionalParamsMapper> {
        None
    }

    fn entra_token_provider(
        &self,
        _request: EntraRequest<'_>,
    ) -> Option<Arc<dyn CallableTokenProvider>> {
        None
    }

    fn foundry_auth_headers(
        &self,
        _request: FoundryAuthRequest<'_>,
    ) -> Result<Headers, Box<RawException>> {
        Ok(Headers::new())
    }

    async fn send(
        &self,
        branch: Branch<'_>,
        logging: &mut LoggingState,
    ) -> Result<BranchAnswer, Box<RawException>> {
        match branch {
            Branch::OpenAi(request) => self.openai(request, logging).await,
            Branch::Azure(_)
            | Branch::BlackForestLabs(_)
            | Branch::Bedrock(_)
            | Branch::Custom(_)
            | Branch::Foundry(_)
            | Branch::Generic(_) => Err(Box::new(value_failure(
                "the fixture wires only the openai branch",
                None,
                None,
            ))),
        }
    }
}

struct SilentLogger;

impl ImageGenerationLogger for SilentLogger {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}

fn reply_response(reply: &ImageGenerationReply) -> ImageResponse {
    ImageResponse {
        created: reply.created,
        data: Some(
            reply
                .data
                .iter()
                .map(|datum| ImageData {
                    b64_json: datum.b64_json.clone(),
                    revised_prompt: datum.revised_prompt.clone(),
                    url: datum.url.clone(),
                })
                .collect(),
        ),
        usage: Some(ImageUsage {
            input_tokens: reply.usage.input_tokens,
            input_tokens_details: ImageInputTokens {
                image_tokens: reply.usage.input_tokens_details.image_tokens,
                text_tokens: reply.usage.input_tokens_details.text_tokens,
            },
            output_tokens: reply.usage.output_tokens,
            total_tokens: reply.usage.total_tokens,
        }),
    }
}

fn failure_of(failure: &ImageGenerationFailure, model: &str) -> Box<RawException> {
    Box::new(provider_failure(
        failure.status().unwrap_or(500),
        &failure.to_string(),
        model,
        "openai",
        "OpenAIError",
    ))
}

struct HttpRun {
    result: Result<ImageResponse, AimageGenerationError>,
    logging: LoggingState,
    marker: Arc<RequestSlot>,
}

struct HttpCase {
    served: Serving,
    ops: FixtureOps,
    fixtures: Fixtures,
    redactor: Redactor,
    runtime: Runtime,
}

impl HttpCase {
    fn new() -> Self {
        let served = serve(
            reply("200 OK", &[("content-type", "application/json")], ANSWER),
            Duration::ZERO,
        )
        .expect("the fixture serves");
        let fixtures = Fixtures::new();
        let policy = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &fixtures.environment)
                .expect("the proxy policy resolves"),
        );
        let source = ProxiedClient::outbound(&policy).expect("the proxied client builds");
        Self {
            served,
            ops: FixtureOps::new(source),
            fixtures,
            redactor: redactor(),
            runtime: runtime(),
        }
    }

    fn run(&self, invocation: Invocation) -> HttpRun {
        let resolution = self.fixtures.context();
        let identity = CatalogIdentity {
            context: &resolution,
        };
        let settings = Settings::default();
        let plan = ImageGenerationPlan {
            identity: &identity,
            operations: &self.ops,
            settings: &settings,
            environment: &self.fixtures.environment,
            custom_providers: &self.fixtures.names,
            custom_provider_map: &[],
            azure_ad_token_provider: None,
            drop_params: false,
            redactor: &self.redactor,
        };
        let entry = PlanEntry { plan: &plan };
        let facade = AimageGenerationPlan {
            entry: &entry,
            resolution: &resolution,
            redactor: &self.redactor,
        };
        // no ambient scope here, so only the wrapper's own re-entry makes the fixture see the captured slot
        let (marker, mut logging) = self
            .runtime
            .block_on(request_slot_scope(async { logging_state() }));
        let result = self
            .runtime
            .block_on(aimage_generation(&facade, invocation, &mut logging));
        HttpRun {
            result,
            logging,
            marker,
        }
    }
}

fn http_call(api_base: &str) -> Invocation {
    let mut call = call_of(Some("openai/dall-e-2"), "cat");
    call.api_base = Some(api_base.to_owned());
    call.api_key = Some(SecretString::from("sk-test"));
    // the caller's own flag is not what the wrapper forwards
    call.kwargs
        .insert(String::from("aimg_generation"), Json::Bool(false));
    Invocation {
        call,
        model_position: ModelPosition::Keyword,
    }
}

fn assert_http_sent(case: &HttpCase, run: &HttpRun) {
    let seen = case.ops.seen();
    assert_eq!(seen.len(), 1, "generation runs exactly once");
    assert_eq!(seen[0].model.as_deref(), Some("dall-e-2"));
    assert_eq!(seen[0].prompt, "cat");
    assert!(
        seen[0].asynchronous,
        "the wrapper forces aimg_generation=true over the caller's false"
    );
    assert!(seen[0].has_key);
    assert_eq!(
        seen[0].slot,
        SlotCheck::Match,
        "the dispatched call runs inside the request slot logging captured"
    );
    assert!(
        run.logging
            .request_slot
            .as_ref()
            .is_some_and(|slot| Arc::ptr_eq(slot, &run.marker)),
        "the logging state carries the request's own slot"
    );
}

fn assert_request(request: &Request) {
    assert_eq!(request.line, "POST /v1/images/generations HTTP/1.1");
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer sk-test"),
        "the resolved key reaches the provider: {}",
        request.head
    );
    let body: Json = serde_json::from_str(&request.body).expect("the request body is JSON");
    assert_eq!(
        body,
        Json::Object(IndexMap::from([
            (String::from("model"), Json::Text(String::from("dall-e-2"))),
            (String::from("prompt"), Json::Text(String::from("cat"))),
        ]))
    );
}

#[test]
fn async_generation_http() {
    let case = HttpCase::new();
    let api_base = format!("http://{}/v1", case.served.address);
    let run = case.run(http_call(&api_base));
    assert_http_sent(&case, &run);
    assert_eq!(
        run.result.expect("the mock answers a generated image"),
        answered_by_the_mock()
    );
    let request = case.served.finish().expect("the fixture answers");
    assert_request(&request);
}

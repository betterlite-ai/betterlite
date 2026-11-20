use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::rerank_provider_plan::{RerankSettings, loads_rerank_settings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::rerank_provider_plan::call::{RerankCall, ReturnDocuments};
use betterlite_core::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest, provider_failure, value_failure,
};
use betterlite_core::api::rerank_provider_plan::plan::{
    CatalogIdentity, DIAGNOSTIC_PREFIX, RerankException, RerankPlan, rerank,
};
use betterlite_core::api::rerank_provider_plan::selection::SelectedConfig;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, OptionalRerankParams,
    RerankLogging, ResponseTransform, get_optional_rerank_params,
};
use betterlite_provider::api::rerank_http::client::ClientSource;
use betterlite_provider::api::rerank_http::error::{ClientError, RerankHttpError};
use betterlite_provider::api::rerank_http::rerank as rerank_http;
use betterlite_provider::api::rerank_http::{RerankCall as HttpCall, RerankDeps, RerankLogContext};
use betterlite_provider::embedding_rerank_providers::error::FamilyError;
use betterlite_provider::embedding_rerank_providers::rerank::FamilyRerankConfig;
use betterlite_provider::nvidia_nim_rerank::ranking::NvidiaNimRankingConfig;
use betterlite_provider::nvidia_nim_rerank::retrieval::NvidiaNimRerankConfig;
use betterlite_provider::nvidia_nim_rerank::{
    NvidiaNimRerankError, NvidiaNimRerankSelection, select_rerank_config,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::super::error::SdkHttpError;
use super::super::result::SyncDiagnostic;
use super::super::result::kinds::SdkResult;
use super::{Call, Failure, Outcome};

const MOCK: &str = "{mock}";
const DIAGNOSTIC_TARGET: &str = "betterlite_core::api::rerank_provider_plan";
const FIXTURE_INIT: &str = r#"{"model":"rerank-test","call_type":"rerank","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const FIXTURE_NOW: &str = "2026-01-01T00:00:05+00:00";

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment = substituted_environment(env, api_base);
    let document = settings_document(call, api_base)?;
    let settings = loads_rerank_settings(&document, &environment)
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let now = DateTime::parse_from_rfc3339(FIXTURE_NOW)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let operations = Operations {
        environment: &environment,
        config: &loaded,
        source: direct(&environment)?,
        redactor,
        now,
    };
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let identity = CatalogIdentity {
        context: &resolution,
    };
    let plan = RerankPlan {
        identity: &identity,
        operations: &operations,
        settings: &settings,
        environment: &environment,
        redactor: &operations.redactor,
    };
    let mut state = logging_state(&operations.redactor)?;
    let diagnostics = Diagnostics::default();
    let subscription = set_default(registry().with(diagnostics.clone()));
    let outcome = rerank(&plan, core_call(call, api_base), &mut state).await;
    drop(subscription);
    let diagnostics = take(&mut *diagnostics.0.lock().unwrap_or_else(PoisonError::into_inner));
    let value = match outcome {
        Ok(response) => Outcome {
            result: Some(serde_json::to_value(&response).map_err(SdkHttpError::Serialize)?),
            failure: None,
            diagnostics,
        },
        Err(exception) => Outcome {
            result: None,
            failure: Some(failure(&exception)?),
            diagnostics,
        },
    };
    Ok(SdkResult::RerankProviderPlan {
        value: Box::new(value),
    })
}

struct Operations<'a> {
    environment: &'a BTreeMap<String, String>,
    config: &'a Config,
    source: Direct,
    redactor: Redactor,
    now: DateTime<FixedOffset>,
}

impl RerankOperations for Operations<'_> {
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, RawException> {
        let config = plan_config(request.config, request.model, self.config, self.environment)
            .unwrap_or(PlanConfig::Permissive);
        let provider = request.custom_llm_provider.unwrap_or_default();
        get_optional_rerank_params(
            &config,
            OptionalRerankParams {
                model: request.model,
                drop_params: request.drop_params,
                query: request.query,
                documents: request.documents,
                custom_llm_provider: request.custom_llm_provider,
                top_n: request.top_n,
                rank_fields: request.rank_fields,
                return_documents: request.return_documents,
                max_chunks_per_doc: request.max_chunks_per_doc,
                max_tokens_per_doc: request.max_tokens_per_doc,
                instruction: request.instruction,
                non_default_params: Some(request.non_default_params),
            },
        )
        .map_err(|error| value_failure(&error.to_string(), request.model, provider))
    }

    async fn send(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<RerankResponse, RawException> {
        let Some(config) =
            plan_config(request.config, request.model, self.config, self.environment)
        else {
            let message = format!(
                "the {} rerank handler is not wired into the oracle adapter",
                request.provider
            );
            return Err(value_failure(&message, request.model, request.provider));
        };
        let mut http_logging = RerankLogging::default();
        let call = HttpCall {
            model: request.model,
            optional_params: request.optional_params,
            api_base: request.api_base,
            api_key: request.api_key,
            litellm_params: Some(request.litellm_params),
            headers: Some(request.headers),
            timeout: request
                .timeout_seconds
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .map(Duration::from_secs_f64),
            is_async: Json::Bool(request.is_async),
            model_response: RerankResponse::default(),
            logging: &mut http_logging,
        };
        let sink = Sink;
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        let mut log = RerankLogContext {
            state: logging,
            env: &env,
        };
        let deps = RerankDeps {
            config: &config,
            source: &self.source,
            supplied: None,
        };
        match rerank_http(call, deps, &mut log).await {
            Ok(response) => Ok(response),
            Err(RerankHttpError::Provider(error)) => Err(provider_failure(
                error.status_code,
                &error.message,
                request.model,
                request.provider,
                "BaseLLMException",
            )),
            Err(other) => Err(value_failure(
                &other.to_string(),
                request.model,
                request.provider,
            )),
        }
    }
}

enum PlanConfig<'a> {
    Family(FamilyRerankConfig<'a>),
    Retrieval(NvidiaNimRerankConfig),
    Ranking(NvidiaNimRankingConfig),
    Permissive,
}

fn plan_config<'a>(
    selected: SelectedConfig,
    model: &str,
    config: &'a Config,
    environment: &'a BTreeMap<String, String>,
) -> Option<PlanConfig<'a>> {
    let family = |family_config| {
        Some(PlanConfig::Family(FamilyRerankConfig {
            config: family_config,
            cohere: &config.sections.cohere_rerank_settings,
            host: &config.sections.jina_infinity_rerank,
            environment,
        }))
    };
    match selected {
        SelectedConfig::Cohere => family(RerankConfig::Cohere),
        SelectedConfig::CohereV2 => family(RerankConfig::CohereV2),
        SelectedConfig::Infinity => family(RerankConfig::Infinity),
        SelectedConfig::JinaAi => family(RerankConfig::JinaAi),
        SelectedConfig::Voyage => family(RerankConfig::Voyage),
        SelectedConfig::NvidiaRetrieval | SelectedConfig::NvidiaRanking => {
            Some(match select_rerank_config(model) {
                NvidiaNimRerankSelection::Retrieval(config) => PlanConfig::Retrieval(config),
                NvidiaNimRerankSelection::Ranking(config) => PlanConfig::Ranking(config),
            })
        }
        SelectedConfig::AzureAi
        | SelectedConfig::HostedVllm
        | SelectedConfig::HuggingFace
        | SelectedConfig::Deepinfra
        | SelectedConfig::VertexAi
        | SelectedConfig::FireworksAi
        | SelectedConfig::Watsonx
        | SelectedConfig::Dashscope
        | SelectedConfig::Qwencloud
        | SelectedConfig::QwenAiPlatform => None,
    }
}

#[derive(Debug, Error)]
enum PlanError {
    #[error(transparent)]
    Family(#[from] FamilyError),
    #[error(transparent)]
    Nvidia(#[from] NvidiaNimRerankError),
}

impl BaseRerankConfig for PlanConfig<'_> {
    type Error = PlanError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        match self {
            Self::Family(config) => Ok(config.validate_environment(request)?),
            Self::Retrieval(config) => Ok(config.validate_environment(request)?),
            Self::Ranking(config) => Ok(config.validate_environment(request)?),
            Self::Permissive => Ok(request.headers.clone()),
        }
    }

    fn transform_rerank_request(
        &self,
        model: &str,
        optional_rerank_params: &Params,
        headers: &Headers,
        litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        match self {
            Self::Family(config) => Ok(config.transform_rerank_request(
                model,
                optional_rerank_params,
                headers,
                litellm_params,
            )?),
            Self::Retrieval(config) => Ok(config.transform_rerank_request(
                model,
                optional_rerank_params,
                headers,
                litellm_params,
            )?),
            Self::Ranking(config) => Ok(config.transform_rerank_request(
                model,
                optional_rerank_params,
                headers,
                litellm_params,
            )?),
            Self::Permissive => Ok(Params::new()),
        }
    }

    fn transform_rerank_response(
        &self,
        model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        match self {
            Self::Family(config) => Ok(config.transform_rerank_response(model_response, request)?),
            Self::Retrieval(config) => {
                Ok(config.transform_rerank_response(model_response, request)?)
            }
            Self::Ranking(config) => Ok(config.transform_rerank_response(model_response, request)?),
            Self::Permissive => Ok(model_response),
        }
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        optional_params: Option<&Params>,
    ) -> String {
        match self {
            Self::Family(config) => config.get_complete_url(api_base, model, optional_params),
            Self::Retrieval(config) => config.get_complete_url(api_base, model, optional_params),
            Self::Ranking(config) => config.get_complete_url(api_base, model, optional_params),
            Self::Permissive => api_base.unwrap_or_default().to_owned(),
        }
    }

    fn supported_rerank_params(&self, model: &str) -> Vec<String> {
        match self {
            Self::Family(config) => config.supported_rerank_params(model),
            Self::Retrieval(config) => config.supported_rerank_params(model),
            Self::Ranking(config) => config.supported_rerank_params(model),
            Self::Permissive => Vec::new(),
        }
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        match self {
            Self::Family(config) => Ok(config.map_cohere_rerank_params(request)?),
            Self::Retrieval(config) => Ok(config.map_cohere_rerank_params(request)?),
            Self::Ranking(config) => Ok(config.map_cohere_rerank_params(request)?),
            Self::Permissive => Ok(Params::new()),
        }
    }
}

// The client the handler would build; the case env carries the proxy settings
struct Direct(ProxiedClient);

impl ClientSource for Direct {
    type Client = ProxiedClient;

    fn client(&self) -> Result<Self::Client, ClientError> {
        Ok(self.0.clone())
    }
}

fn direct(env: &BTreeMap<String, String>) -> Result<Direct, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    Ok(Direct(client))
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit = serde_json::from_str(FIXTURE_INIT).map_err(SdkHttpError::Serialize)?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .map_err(SdkHttpError::LoggingState)
}

fn core_call(call: &Call, api_base: &str) -> RerankCall {
    RerankCall {
        model: call.model.clone(),
        query: call.query.clone(),
        documents: call.documents.clone(),
        custom_llm_provider: call.custom_llm_provider.clone(),
        top_n: call.top_n,
        rank_fields: call.rank_fields.clone(),
        return_documents: match call.return_documents {
            None => ReturnDocuments::Default,
            Some(None) => ReturnDocuments::Null,
            Some(Some(value)) => ReturnDocuments::Value(value),
        },
        max_chunks_per_doc: call.max_chunks_per_doc,
        max_tokens_per_doc: call.max_tokens_per_doc,
        kwargs: call
            .kwargs
            .iter()
            .map(|(name, value)| (name.clone(), substituted_json(value, api_base)))
            .collect(),
    }
}

fn settings_document(call: &Call, api_base: &str) -> Result<String, SdkHttpError> {
    #[derive(serde::Serialize)]
    struct Document<'a> {
        litellm_settings: &'a super::Settings,
    }
    let text = serde_json::to_string(&Document {
        litellm_settings: &call.litellm_settings,
    })
    .map_err(SdkHttpError::Serialize)?;
    Ok(text.replace(MOCK, api_base))
}

fn substituted_environment(
    env: &BTreeMap<String, String>,
    api_base: &str,
) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), value.replace(MOCK, api_base)))
        .collect()
}

fn substituted_json(value: &Json, api_base: &str) -> Json {
    match value {
        Json::Text(text) => Json::Text(text.replace(MOCK, api_base)),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| substituted_json(item, api_base))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), substituted_json(item, api_base)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}

// The exception the SDK raises for this class, as the pinned litellm.exceptions module names it
fn failure(exception: &RerankException) -> Result<Failure, SdkHttpError> {
    let metadata = exception.class.metadata();
    Ok(Failure {
        class: class_name(&exception.class)?.to_owned(),
        status: Some(metadata.status_code),
        kind: text_field(metadata.error_type),
        param: text_field(metadata.param),
        code: metadata.openai_code,
        message: exception.message.clone(),
    })
}

fn text_field(field: WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text),
        WireField::Missing(_) | WireField::Null => None,
    }
}

fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::Authentication(_) => Ok("AuthenticationError"),
        ErrorClass::NotFound(_) => Ok("NotFoundError"),
        ErrorClass::PermissionDenied(_) => Ok("PermissionDeniedError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        ErrorClass::UnprocessableEntity(_) => Ok("UnprocessableEntityError"),
        ErrorClass::ContextWindowExceeded(_) => Ok("ContextWindowExceededError"),
        ErrorClass::ContentPolicyViolation(_) => Ok("ContentPolicyViolationError"),
        ErrorClass::ServiceUnavailable(_) => Ok("ServiceUnavailableError"),
        ErrorClass::BadGateway(_) => Ok("BadGatewayError"),
        ErrorClass::InternalServer(_) => Ok("InternalServerError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::Api(_) => Ok("APIError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::UnsupportedParams(_) => Ok("UnsupportedParamsError"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}

#[derive(Clone, Default)]
struct Diagnostics(Arc<Mutex<Vec<SyncDiagnostic>>>);

#[derive(Default)]
struct MessageVisitor(Option<String>);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event.metadata().target().starts_with(DIAGNOSTIC_TARGET) {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let Some(message) = visitor.0 else {
            return;
        };
        if !message.starts_with(DIAGNOSTIC_PREFIX) {
            return;
        }
        let level = match *event.metadata().level() {
            Level::ERROR => "error",
            Level::WARN => "warning",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SyncDiagnostic {
                level: level.to_owned(),
                prefix: DIAGNOSTIC_PREFIX.to_owned(),
            });
    }
}

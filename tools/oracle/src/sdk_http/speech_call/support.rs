use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::speech_call::Settings;
use betterlite_context::media_config_selection::text_to_speech::TextToSpeechConfigTag;
use betterlite_core::api::speech_call::call::SpeechCall;
use betterlite_core::api::speech_call::error::{SpeechCallError, SpeechException, SpeechFailure};
use betterlite_core::api::speech_call::operations::{
    SpeechConfigOperation, SpeechConfigSource, SpeechOperation, SpeechOperations,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::super::result::{Failure, Nullable};
use super::{API_BASE_SENTINEL, Globals, Step};

// a base or a key written as the sentinel takes the harness mock's origin
pub(super) fn environment(
    env: &BTreeMap<String, String>,
    api_base: &str,
) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| {
            let resolved = if value == API_BASE_SENTINEL {
                api_base.to_owned()
            } else {
                value.clone()
            };
            (name.clone(), resolved)
        })
        .collect()
}

pub(super) fn settings(
    globals: &Globals,
    environment: &BTreeMap<String, String>,
    api_base: &str,
) -> Result<Settings, SdkHttpError> {
    let base = globals
        .api_base
        .as_deref()
        .map(|value| harness(value, api_base));
    let source = serde_json::json!({
        "litellm_settings": {
            "api_base": base,
            "api_key": globals.api_key,
            "openai_key": globals.openai_key,
            "organization": globals.organization,
            "project": globals.project,
            "azure_key": globals.azure_key,
            "api_version": globals.api_version,
            "num_retries": globals.num_retries,
            "request_timeout": globals.request_timeout,
        }
    });
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source.to_string())
    .map(|config| config.sections.speech_call)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))
}

fn harness(value: &str, api_base: &str) -> String {
    if value == API_BASE_SENTINEL {
        api_base.to_owned()
    } else {
        value.to_owned()
    }
}

// the resolution context a case runs with: no catalog entries and no custom providers
pub(super) struct Fixture {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    globals: GlobalCredentials,
}

impl Fixture {
    pub(super) fn new(
        environment: &BTreeMap<String, String>,
        globals: &Globals,
        api_base: &str,
    ) -> Result<Self, SdkHttpError> {
        Ok(Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Reject)
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
            names: BTreeSet::new(),
            environment: environment.clone(),
            globals: GlobalCredentials {
                api_key: globals.api_key.clone().map(SecretString::from),
                api_base: globals
                    .api_base
                    .as_deref()
                    .map(|value| harness(value, api_base)),
                ..GlobalCredentials::default()
            },
        })
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: self.globals.clone(),
        }
    }
}

pub(super) fn speak(step: &Step, api_base: &str) -> Result<SpeechCall, SdkHttpError> {
    Ok(SpeechCall {
        model: step.model.clone(),
        input: step.input.clone(),
        voice: voice(step.voice.as_ref())?,
        api_key: step.api_key.as_deref().map(SecretString::from),
        api_base: step
            .api_base
            .as_deref()
            .map(|value| harness(value, api_base)),
        api_version: step.api_version.clone(),
        organization: step.organization.clone(),
        project: step.project.clone(),
        custom_llm_provider: step.custom_llm_provider.clone(),
        base_url: step
            .base_url
            .as_deref()
            .map(|value| harness(value, api_base)),
        max_retries: step.max_retries,
        timeout_seconds: step
            .timeout_seconds
            .map(|seconds| Duration::from_secs(seconds).as_secs_f64()),
        metadata: step.metadata.clone(),
        response_format: step.response_format.clone(),
        speed: step
            .speed
            .map(|speed| {
                Decimal::try_from(speed).map_err(|error| SdkHttpError::Invalid(error.to_string()))
            })
            .transpose()?,
        instructions: step.instructions.clone(),
        headers: step.headers.clone(),
        client: None,
        shared_session: None,
        aspeech: false,
        kwargs: step.kwargs.clone(),
    })
}

fn voice(voice: Option<&Json>) -> Result<Option<SpeechVoice>, SdkHttpError> {
    voice
        .map(|voice| {
            let wire = serde_json::to_value(voice).map_err(SdkHttpError::Serialize)?;
            serde_json::from_value(wire).map_err(|error| {
                SdkHttpError::Invalid(format!("the voice is not a string or an object: {error}"))
            })
        })
        .transpose()
}

pub(super) fn logging() -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"tts-1","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let redactor = redactor()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated-trace"),
        Duration::default,
    )
    .map_err(SdkHttpError::LoggingState)
}

pub(super) fn redactor() -> Result<Redactor, SdkHttpError> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)
}

// the Lyria catalog answer the case scripts; no config object handles a tag, which is B157's absent case
pub(super) struct StepConfigs {
    pub(super) lyria: bool,
}

impl SpeechConfigSource for StepConfigs {
    fn is_lyria_model(&self, _model: &str) -> bool {
        self.lyria
    }

    fn config(&self, _tag: TextToSpeechConfigTag) -> Option<&dyn SpeechConfigOperation> {
        None
    }
}

// the B010 seam this task dispatches into; the named provider owners build the real operations
pub(super) struct CannedOperations {
    pub(super) body: Vec<u8>,
}

impl SpeechOperations for CannedOperations {
    fn execute(
        &self,
        _operation: SpeechOperation<'_>,
    ) -> impl Future<Output = Result<BinaryContent, SpeechCallError>> + Send {
        ready(Ok(BinaryContent {
            status: 200,
            headers: IndexMap::from([(String::from("content-type"), String::from("audio/wav"))]),
            body: self.body.clone(),
        }))
    }
}

// the failure a speech() call raises, as the SDK caller observes it
pub(super) fn failure(error: &SpeechException) -> Result<Failure, SdkHttpError> {
    match &error.failure {
        SpeechFailure::Class { class } => Ok(Failure {
            class: (*class).to_owned(),
            status: 0,
            kind: None,
            param: None,
            code: None,
            message: error.message.clone(),
        }),
        SpeechFailure::Mapped(class) => Ok(Failure {
            class: class_name(class)?.to_owned(),
            status: class.metadata().status_code,
            kind: field(class.metadata().error_type),
            param: field(class.metadata().param),
            code: class.metadata().openai_code,
            message: error.message.clone(),
        }),
    }
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// only the classes the pinned SDK raises as openai.APIError subclasses have a projection here
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
        other @ (ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_)) => Err(SdkHttpError::Unprojected(format!("{other:?}"))),
    }
}

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_core::api::speech_chat_bridge::arguments::SpeechBridgeArguments;
use betterlite_core::api::speech_chat_bridge::error::{SpeechBridgeError, SpeechTransformError};
use betterlite_core::api::speech_chat_bridge::response::SpeechBinaryResponse;
use betterlite_core::api::speech_chat_bridge::{
    SpeechCompletionAnswer, SpeechCompletionCall, SpeechCompletionOutput, SpeechCompletions,
    SpeechToCompletionBridge,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::api::speech_chat_bridge::audio_base64::SpeechAudioError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::files::records::BinaryContent;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::call_checks::validate_timeout;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::native;
use super::result::Failure;
use super::result::kinds::SdkResult;
use super::{OpenaiChat, OracleLimits};

const CONTENT_TYPE_HEADER: &str = "content-type";

// the speech bridge call: everything the LiteLLM handler takes, plus the deployment the native side needs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    model: String,
    input: String,
    #[serde(default)]
    voice: Option<Json>,
    #[serde(default)]
    optional_params: Option<Json>,
    #[serde(default)]
    litellm_params: Option<Json>,
    #[serde(default)]
    headers: Option<Json>,
    custom_llm_provider: String,
    api_key: String,
    timeout_seconds: u64,
}

// the binary answer the /v1/audio/speech route would return
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl From<SpeechBinaryResponse> for Outcome {
    fn from(answer: SpeechBinaryResponse) -> Self {
        let BinaryContent {
            status,
            headers,
            body,
        } = answer.content;
        Self {
            status,
            content_type: headers.get(CONTENT_TYPE_HEADER).cloned(),
            body,
        }
    }
}

// the completion seam answers with the provider's response or the failure the SDK layer projects
#[derive(Debug, Error)]
enum CompletionFailure {
    #[error("the provider answered with a projected failure")]
    Provider { failure: Box<Failure> },
    #[error(transparent)]
    Sdk(#[from] SdkHttpError),
}

struct BridgeCompletions {
    api_base: String,
    api_key: String,
    timeout_seconds: u64,
    env: BTreeMap<String, String>,
}

impl SpeechCompletions for BridgeCompletions {
    type Error = CompletionFailure;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, CompletionFailure>> + Send {
        let SpeechCompletionCall { kwargs, .. } = call;
        let api_base = self.api_base.clone();
        let env = self.env.clone();
        let api_key = self.api_key.clone();
        let timeout_seconds = self.timeout_seconds;
        async move {
            let model = projected_model(&kwargs)?;
            let request = bridge_request(&kwargs)?;
            let chat = OpenaiChat {
                model: model.clone(),
                messages: Vec::new(),
                api_key,
                timeout_seconds,
            };
            let executor = native::executor(&chat, &api_base, &env)?;
            let deployment = executor
                .deployments
                .first()
                .ok_or_else(|| SdkHttpError::Invalid(format!("no deployment serves {model}")))?;
            match executor.send(deployment, &request).await {
                Ok(answer) => Ok(SpeechCompletionOutput::Response(Box::new(
                    SpeechCompletionAnswer {
                        response: answer.response,
                        hidden_params: HiddenParams::new(),
                    },
                ))),
                Err(ChatError::Provider { response }) => {
                    let failure =
                        native::failure(&model, &response).map_err(CompletionFailure::Sdk)?;
                    Err(CompletionFailure::Provider {
                        failure: Box::new(failure),
                    })
                }
                Err(other) => Err(CompletionFailure::Sdk(SdkHttpError::Chat(Box::new(other)))),
            }
        }
    }
}

// the model litellm_params can still override, so the deployment is built from the projected kwargs
fn projected_model(kwargs: &IndexMap<String, Json>) -> Result<String, SdkHttpError> {
    match kwargs.get("model") {
        Some(Json::Text(model)) => Ok(model.clone()),
        Some(_) | None => Err(SdkHttpError::Invalid(
            "the projected model must be text".to_owned(),
        )),
    }
}

// the completion pipeline reads the request body the way the server does, without the bridge-only keys
fn bridge_request(kwargs: &IndexMap<String, Json>) -> Result<ChatRequest, SdkHttpError> {
    let mut wire = kwargs.clone();
    wire.shift_remove("headers");
    wire.shift_remove("custom_llm_provider");
    let body = serde_json::to_vec(&wire).map_err(SdkHttpError::Serialize)?;
    ChatRequest::parse(&body).map_err(|source| SdkHttpError::ChatParse(Box::new(source)))
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate_timeout(self.timeout_seconds, limits)
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        let completions = BridgeCompletions {
            api_base: api_base.to_owned(),
            api_key: self.api_key.clone(),
            timeout_seconds: self.timeout_seconds,
            env: env.clone(),
        };
        async move { self.bridge(completions).await }
    }
}

impl Call {
    async fn bridge(&self, completions: BridgeCompletions) -> Result<SdkResult, SdkHttpError> {
        let mut logging = bridge_logging()?;
        let model = Json::Text(self.model.clone());
        let input = Json::Text(self.input.clone());
        let provider = Json::Text(self.custom_llm_provider.clone());
        let arguments = SpeechBridgeArguments {
            model: Some(&model),
            input: Some(&input),
            voice: self.voice.as_ref(),
            optional_params: self.optional_params.as_ref(),
            litellm_params: self.litellm_params.as_ref(),
            headers: self.headers.as_ref(),
            custom_llm_provider: Some(&provider),
        };
        let bridge = SpeechToCompletionBridge { completions };
        match bridge.speech(arguments, Some(&mut logging)).await {
            Ok(answer) => Ok(SdkResult::SpeechChatBridge {
                value: Box::new(Outcome::from(answer)),
            }),
            Err(SpeechBridgeError::Completion { source }) => match source {
                CompletionFailure::Provider { failure } => Ok(SdkResult::Failure(*failure)),
                CompletionFailure::Sdk(error) => Err(error),
            },
            Err(SpeechBridgeError::Required { name }) => {
                Ok(SdkResult::Failure(required_failure(name)))
            }
            Err(SpeechBridgeError::Transform(error)) => {
                Ok(SdkResult::Failure(transform_failure(&error)))
            }
        }
    }
}

fn bridge_logging() -> Result<LoggingState, SdkHttpError> {
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    LoggingState::new(
        LoggingInit {
            model: String::from("gpt-4o-audio-preview"),
            call_type: String::from("completion"),
            start_time: DateTime::UNIX_EPOCH.fixed_offset(),
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
        &redactor,
        || String::from("generated-trace"),
        Duration::default,
    )
    .map_err(SdkHttpError::LoggingState)
}

fn required_failure(name: &str) -> Failure {
    Failure {
        class: String::from("ValueError"),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: format!("{name} is required"),
    }
}

// LiteLLM raises plain exceptions for these, so the class is the exception type the Python side would report
fn transform_failure(error: &SpeechTransformError) -> Failure {
    match error {
        SpeechTransformError::UnsupportedResponseFormat { .. } => error.error_fields().map_or_else(
            || named_failure("BadRequestError", error),
            |fields| Failure {
                class: String::from("BadRequestError"),
                status: fields.status,
                kind: Some(fields.error_type.map(String::from)),
                param: Some(fields.param.map(String::from)),
                code: Some(fields.code.map(String::from)),
                message: format!("litellm.BadRequestError: {error}"),
            },
        ),
        SpeechTransformError::MissingAudio => named_failure("ValueError", error),
        SpeechTransformError::NoChoices => named_failure("IndexError", error),
        SpeechTransformError::Unmapped { .. } => named_failure("Exception", error),
        SpeechTransformError::Audio(SpeechAudioError::NonAscii) => {
            named_failure("ValueError", error)
        }
        SpeechTransformError::Audio(_) => named_failure("Error", error),
        SpeechTransformError::AudioPart { .. } => named_failure("ValidationError", error),
        SpeechTransformError::WavTooLarge => named_failure("error", error),
    }
}

fn named_failure(class: &str, error: &SpeechTransformError) -> Failure {
    Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}

mod tests;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::transcription_call::Settings;
use betterlite_core::api::transcription_call::call::TranscriptionCall;
use betterlite_core::api::transcription_call::dispatch::TranscriptionAnswer;
use betterlite_core::api::transcription_call::error::{
    AsyncTranscriptionError, TranscriptionError,
};
use betterlite_core::api::transcription_call::service::{DurationDecoder, Transcriptions};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::audio_file_input::{AudioFile, AudioTuple, TupleContent};
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::vector_query_embedding::outbound_client;

mod openai;
mod support;
mod xai;

use self::support::{Dispatch, DispatchFailure};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}

// litellm.transcription's arguments, as the case files write them; the harness supplies the api_base
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub missing_model: bool,
    pub file: File,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub timestamp_granularities: Option<Vec<String>>,
    #[serde(default)]
    pub temperature: Option<i64>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub azure_ad_token: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<Headers>,
    #[serde(default)]
    pub params: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub filename: String,
    pub content: String,
}

// What both SDKs hand back for one call: the public record, the duration main.py hides, or the failure
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub text: Option<String>,
    pub words: Option<Json>,
    pub public_duration: Option<Json>,
    pub hidden_duration: Option<Json>,
    pub status: Option<u16>,
    pub raised: Option<Raised>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Raised {
    pub class: String,
    pub message: String,
}

impl Outcome {
    fn raised(class: &str, message: &str) -> Self {
        Self {
            text: None,
            words: None,
            public_duration: None,
            hidden_duration: None,
            status: None,
            raised: Some(Raised {
                class: class.to_owned(),
                message: message.to_owned(),
            }),
        }
    }

    fn status(status: u16) -> Self {
        Self {
            text: None,
            words: None,
            public_duration: None,
            hidden_duration: None,
            status: Some(status),
            raised: None,
        }
    }
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        execute(self, api_base, env).await
    }
}

async fn execute(
    input: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let settings = settings(env)?;
    let catalog = BundledCatalog::default().get();
    let names = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let client = outbound_client(env)?;
    let dispatch = Dispatch { client: &client };
    let files = support::Files;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let call = transcription_call(input, api_base);
    let service = Transcriptions {
        dispatch: &dispatch,
        settings: &settings,
        resolution,
        bundled: &catalog,
        files: &files,
        decoder: DurationDecoder::Unavailable,
        redactor: &redactor,
        logging: None,
        identify: uuid::Uuid::new_v4,
    };
    let outcome = match input.mode {
        Mode::Sync => sync_outcome(service.transcription(&call, audio(input)).await)?,
        Mode::Async => async_outcome(input, &call, audio(input), &service).await?,
    };
    Ok(SdkResult::TranscriptionCall {
        value: Box::new(outcome),
    })
}

fn settings(env: &BTreeMap<String, String>) -> Result<Settings, SdkHttpError> {
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let config = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.transcription_call)
}

fn transcription_call(input: &Call, api_base: &str) -> TranscriptionCall {
    TranscriptionCall {
        model: input.model.clone().unwrap_or_default(),
        language: input.language.clone(),
        prompt: input.prompt.clone(),
        response_format: input.response_format.clone(),
        timestamp_granularities: input.timestamp_granularities.clone(),
        temperature: input.temperature,
        user: input.user.clone(),
        timeout: input.timeout_seconds,
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: Some(api_base.to_owned()),
        api_version: input.api_version.clone(),
        max_retries: input.max_retries,
        custom_llm_provider: input.custom_llm_provider.clone(),
        base_url: input.base_url.clone(),
        atranscription: None,
        client: None,
        extra_headers: input.extra_headers.clone(),
        shared_session: None,
        azure_ad_token: input.azure_ad_token.clone().map(SecretString::from),
        params: input.params.clone(),
    }
}

fn audio(input: &Call) -> AudioFile<'_> {
    AudioFile::Tuple(AudioTuple {
        filename: &input.file.filename,
        content: Some(TupleContent::Bytes(input.file.content.as_bytes())),
    })
}

fn sync_outcome(
    answer: Result<TranscriptionAnswer, TranscriptionError<DispatchFailure>>,
) -> Result<Outcome, SdkHttpError> {
    match answer {
        Ok(TranscriptionAnswer::Reply(reply)) => Ok(outcome_of(&reply.response)),
        Ok(TranscriptionAnswer::Mapping(_)) | Ok(TranscriptionAnswer::Unexpected(_)) => {
            Err(SdkHttpError::Invalid(String::from(
                "the sync transcription answered a mocked value",
            )))
        }
        Err(TranscriptionError::UnmappedProvider) => Ok(Outcome::raised(
            "ValueError",
            "Unmapped provider passed in. Unable to get the response.",
        )),
        Err(TranscriptionError::Dispatch(DispatchFailure::Provider { status, .. })) => status
            .map_or_else(
                || {
                    Err(SdkHttpError::Invalid(String::from(
                        "the provider failure has no status",
                    )))
                },
                |status| Ok(Outcome::status(status)),
            ),
        Err(error) => Err(SdkHttpError::Invalid(error.to_string())),
    }
}

async fn async_outcome(
    input: &Call,
    call: &TranscriptionCall,
    file: AudioFile<'_>,
    service: &Transcriptions<'_, Dispatch<'_>>,
) -> Result<Outcome, SdkHttpError> {
    let model = if input.missing_model {
        None
    } else {
        Some(call.model.as_str())
    };
    match service.atranscription(model, call, file, true).await {
        Ok(response) => Ok(outcome_of(&response)),
        Err(AsyncTranscriptionError::MissingModel) => Ok(Outcome::raised("KeyError", "'model'")),
        Err(AsyncTranscriptionError::Exception(exception)) => {
            Err(SdkHttpError::Invalid(exception.message))
        }
    }
}

fn outcome_of(response: &TranscriptionResponse) -> Outcome {
    Outcome {
        text: response.text.clone(),
        words: field(response, "words"),
        public_duration: field(response, "duration"),
        hidden_duration: hidden_duration(response),
        status: None,
        raised: None,
    }
}

fn field(response: &TranscriptionResponse, name: &str) -> Option<Json> {
    response
        .extra
        .get(name)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn hidden_duration(response: &TranscriptionResponse) -> Option<Json> {
    response
        .extra
        .get("_hidden_params")?
        .get("audio_transcription_duration")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

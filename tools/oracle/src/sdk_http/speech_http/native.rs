use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_store_http_write::service::ClientSource as _;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::speech_http as production;
use betterlite_provider::api::speech_http::error::{SpeechError, SpeechProviderError};
use betterlite_provider::api::speech_provider_contract::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, Source, source};
use super::{BinaryValue, Call, Failure, Outcome, Step, StepOutcome, Transformed};

const MODEL: &str = "speech-model";

// The production handlers over the mock's base: one pooled client and one logging state per step
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&config, env)?;
    let mut outcomes = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        outcomes.push(run_step(step, api_base, &origin).await?);
    }
    Ok(SdkResult::SpeechHttp {
        value: Box::new(Outcome { steps: outcomes }),
    })
}

async fn run_step(
    step: &Step,
    api_base: &str,
    origin: &Source,
) -> Result<StepOutcome, SdkHttpError> {
    let provider = StepConfig::new(step);
    let params = production::SpeechParams {
        api_key: step.api_key.as_deref().map(SecretString::from),
        api_base: Some(api_base.to_owned()),
        ssl_verify: step.ssl_verify,
    };
    let litellm_params = litellm_params(api_base);
    let extra = step.extra_headers.as_ref().map(headers_map);
    let call = production::SpeechCall {
        model: MODEL,
        input: &step.input,
        voice: step.voice.as_deref(),
        optional_params: &step.optional_params,
        litellm_params: &litellm_params,
        extra_headers: extra.as_ref(),
        params: &params,
        timeout: step.timeout_seconds.map(Duration::from_secs),
    };
    let pooled = origin
        .client(step.ssl_verify)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let supplied = step.supplied.map(|kind| production::SuppliedClient {
        kind: kind.client_kind(),
        transport: &pooled,
    });
    let deps = production::SpeechDeps {
        config: &provider,
        source: origin,
        supplied,
    };
    let mut fixture = Fixture::new()?;
    let answer = fixture.call(&deps, step.mode.client_kind(), &call).await;
    outcome(answer)
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    answer: Result<production::SpeechBinaryResponse, SpeechError>,
) -> Result<StepOutcome, SdkHttpError> {
    match answer {
        Ok(response) => Ok(StepOutcome {
            value: Some(binary(response.content)),
            failure: None,
        }),
        Err(SpeechError::Provider(error)) => Ok(StepOutcome {
            value: None,
            failure: Some(failure(error)),
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn binary(content: BinaryContent) -> BinaryValue {
    BinaryValue {
        status: content.status,
        body: content.body,
        headers: content.headers.into_iter().collect(),
    }
}

// Every speech config LiteLLM ships returns its provider's exception class; the injected one returns the base class
fn failure(error: SpeechProviderError) -> Failure {
    Failure {
        class: "BaseLLMException".to_owned(),
        status: error.status_code,
        synthesized: error.status_code_is_synthesized,
        message: error.message,
        headers: error.headers.into_iter().collect(),
    }
}

// The dict the handler reads ssl_verify and api_key from; the key travels in SpeechParams, never as plain JSON
fn litellm_params(api_base: &str) -> Object {
    let mut params = Object::new();
    params.insert("api_base".to_owned(), Value::String(api_base.to_owned()));
    params
}

fn headers_map(headers: &BTreeMap<String, String>) -> SpeechHeaders {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// The injected speech config: the case's validated headers and request data, with no transform of its own
struct StepConfig {
    validated: SpeechHeaders,
    data: TextToSpeechRequestData,
}

impl StepConfig {
    fn new(step: &Step) -> Self {
        Self {
            validated: step
                .validated
                .as_ref()
                .map_or_else(SpeechHeaders::new, headers_map),
            data: request_data(&step.transformed),
        }
    }
}

fn request_data(transformed: &Transformed) -> TextToSpeechRequestData {
    TextToSpeechRequestData {
        dict_body: transformed.dict_body.clone(),
        ssml_body: transformed.ssml_body.clone(),
        headers: transformed.headers.as_ref().map(headers_map),
    }
}

impl TextToSpeechConfig for StepConfig {
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::new()
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        _optional_params: Object,
        _options: &SpeechMappingOptions,
    ) -> (Option<String>, Object) {
        (None, Object::new())
    }

    fn validate_environment(
        &self,
        _headers: &SpeechHeaders,
        _model: &str,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> SpeechHeaders {
        self.validated.clone()
    }

    fn speech_request(&self, _request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        self.data.clone()
    }

    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        BinaryContent::from_response(raw_response)
    }
}

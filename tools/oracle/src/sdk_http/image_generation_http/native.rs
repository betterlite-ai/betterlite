use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams, ImageResponse};
use betterlite_openai::error::ProviderError;
use betterlite_provider::api::image_contracts::error::ImageContractError;
use betterlite_provider::api::image_contracts::generation::{
    BaseImageGenerationConfig, ImageGenerationReply, ImageGenerationRequest,
};
use betterlite_provider::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest};
use betterlite_provider::api::image_generation_http::error::ImageGenerationError;
use betterlite_provider::api::image_generation_http::log::{ImageGenerationLogger, PreCallRecord};
use betterlite_provider::api::image_generation_http::{
    ImageGenerationCall, ImageGenerationDeps, ImageGenerationOutcome, prepare, send,
};
use secrecy::ExposeSecret as _;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Source, source};
use super::{
    Call, Effects, Failure, Outcome, PreCallEffect, RequestEffect, ResponseEffect, UrlEffect,
    ValidateEffect,
};

// The production handler over the mock's base: the same config section and pooled client a request would build
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
    let fixture = Fixture::new(call.multipart);
    let params = litellm_params(call, api_base);
    let generation = ImageGenerationCall {
        model: &call.model,
        prompt: &call.prompt,
        optional_params: &call.optional_params,
        litellm_params: &params,
        extra_headers: call.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        fake_stream: call.fake_stream,
        litellm_metadata: call.litellm_metadata.as_ref(),
        api_key: call.api_key.as_deref(),
        timeout: Duration::from_secs(call.timeout_seconds),
    };
    let deps = ImageGenerationDeps {
        config: &fixture,
        source: &origin,
        supplied: None,
    };
    let mut logger = Logger { fixture: &fixture };
    let prepared = prepare(&deps, call.mode.client_kind(), &generation, &mut logger)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let answer = send(prepared).await;
    let value = outcome(answer, fixture.effects()?)?;
    Ok(SdkResult::ImageGenerationHttp {
        value: Box::new(value),
    })
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    answer: Result<ImageGenerationOutcome, ImageGenerationError>,
    effects: Effects,
) -> Result<Outcome, SdkHttpError> {
    match answer {
        Ok(outcome) => Ok(Outcome {
            result: Some(serde_json::to_value(&outcome.response).map_err(SdkHttpError::Serialize)?),
            failure: None,
            provider_headers: Some(outcome.provider_headers.headers.into_iter().collect()),
            effects,
        }),
        Err(ImageGenerationError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(failure(*error)),
            provider_headers: None,
            effects,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

// The deployment map a caller hands the handler: the mock base, then the key and certificate setting when set
fn litellm_params(call: &Call, api_base: &str) -> ImageParams {
    let mut params = ImageParams::new();
    params.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    if let Some(key) = &call.litellm_params.api_key {
        params.insert(String::from("api_key"), Json::Text(key.clone()));
    }
    if let Some(setting) = call.litellm_params.ssl_verify {
        params.insert(String::from("ssl_verify"), Json::Bool(setting));
    }
    params
}

// The exception _handle_error raises, as much of it as the SDK's callers read
fn failure(error: ProviderError) -> Failure {
    Failure {
        status: error.status_code,
        synthesized: error.status_code_is_synthesized,
        message: error.message,
        headers: error.headers.map_or_else(BTreeMap::new, |pairs| {
            pairs
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect()
        }),
    }
}

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] ImageContractError),
    #[error("the reply body is not an image response")]
    Reply(#[source] serde_json::Error),
}

#[derive(Default)]
struct Recorded {
    validate: Option<ValidateEffect>,
    url: Option<UrlEffect>,
    request: Option<RequestEffect>,
    response: Option<ResponseEffect>,
    pre_call: Option<PreCallEffect>,
}

// The config LiteLLM's finite test setup would use: it records what each method is handed and answers as a config can
struct Fixture {
    multipart: bool,
    recorded: Mutex<Recorded>,
}

impl Fixture {
    fn new(multipart: bool) -> Self {
        Self {
            multipart,
            recorded: Mutex::new(Recorded::default()),
        }
    }

    fn recorded(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn effects(&self) -> Result<Effects, SdkHttpError> {
        let recorded = self.recorded();
        Ok(Effects {
            validation: required(recorded.validate.as_ref(), "validate_environment")?,
            url: required(recorded.url.as_ref(), "get_complete_url")?,
            request: required(
                recorded.request.as_ref(),
                "transform_image_generation_request",
            )?,
            response: recorded.response.clone(),
            pre_call: required(recorded.pre_call.as_ref(), "pre_call")?,
        })
    }
}

fn required<T: Clone>(slot: Option<&T>, step: &str) -> Result<T, SdkHttpError> {
    slot.cloned()
        .ok_or_else(|| SdkHttpError::Invalid(format!("the fixture never ran {step}")))
}

impl BaseImageGenerationConfig for Fixture {
    type Error = FixtureError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _non_default_params: &ImageParams,
        optional_params: ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        optional_params
    }

    fn validate_environment(&self, request: ImageEnvironment<'_>) -> ImageHeaders {
        let mut recorded = self.recorded();
        recorded.validate = Some(ValidateEffect {
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            api_base: request.api_base.map(str::to_owned),
            headers: header_fields(request.headers),
            model: request.model.to_owned(),
            messages: request.messages.to_vec(),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
        });
        headers([("x", "validated")])
    }

    fn get_complete_url(&self, request: ImageUrlRequest<'_>) -> String {
        let base = request.api_base.unwrap_or_default();
        let mut recorded = self.recorded();
        recorded.url = Some(UrlEffect {
            api_base: request.api_base.map(str::to_owned),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            model: request.model.to_owned(),
            stream: request.stream,
        });
        format!("{base}/images")
    }

    // ast-grep-ignore: no-long-names
    fn transform_image_generation_request(
        &self,
        request: ImageGenerationRequest<'_>,
    ) -> Result<ImageParams, FixtureError> {
        let mut recorded = self.recorded();
        recorded.request = Some(RequestEffect {
            model: request.model.to_owned(),
            prompt: request.prompt.to_owned(),
            headers: header_fields(request.headers),
        });
        Ok(identity_body(request.model, request.prompt))
    }

    // ast-grep-ignore: no-long-names
    fn transform_image_generation_response(
        &self,
        reply: ImageGenerationReply<'_>,
    ) -> Result<ImageResponse, FixtureError> {
        let mut recorded = self.recorded();
        recorded.response = Some(ResponseEffect {
            model: reply.model.to_owned(),
            request_data: reply.request_data.clone(),
            encoding: None,
            api_key: reply.api_key.map(|key| key.expose_secret().to_owned()),
            json_mode: reply.json_mode,
        });
        fill_from_reply(reply.model_response, &reply.raw_response.body)
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }
}

// The identity body: the prompt and the model, which is all the handler's own request assembly adds
fn identity_body(model: &str, prompt: &str) -> ImageParams {
    let entries = [
        (String::from("model"), Json::Text(model.to_owned())),
        (String::from("prompt"), Json::Text(prompt.to_owned())),
    ];
    entries.into_iter().collect()
}

fn header_fields(headers: &ImageHeaders) -> ImageParams {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

fn headers(pairs: &[(&str, &str)]) -> ImageHeaders {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// A provider config fills the response object it is handed; LiteLLM's ImageResponse starts with a zero usage
fn fill_from_reply(target: &mut ImageResponse, body: &[u8]) -> Result<ImageResponse, FixtureError> {
    let parsed: ImageResponse = serde_json::from_slice(body).map_err(FixtureError::Reply)?;
    target.created = parsed.created;
    target.data = parsed.data;
    target.usage = Some(parsed.usage.unwrap_or_default());
    Ok(target.clone())
}

struct Logger<'fixture> {
    fixture: &'fixture Fixture,
}

impl ImageGenerationLogger for Logger<'_> {
    fn pre_call(&mut self, record: PreCallRecord<'_>) {
        let mut recorded = self.fixture.recorded();
        recorded.pre_call = Some(PreCallEffect {
            input: record.input.to_owned(),
            api_key: record.api_key.to_owned(),
            api_base: record.api_base.to_owned(),
            headers: header_fields(record.headers),
            complete_input_dict: record.complete_input_dict.clone(),
        });
    }
}

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};
use betterlite_provider::api::image_edit_http::error::ImageEditHttpError;
use betterlite_provider::api::image_edit_http::service::{
    ImageEditCall, ImageEditDeps, ImageEditOutcome,
};

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, FixtureConfig, FixtureError, source};
use super::{Call, Failure, LoggedCall, Outcome};

// The production handler over the mock's base: the same pooled client and pre-call a request would build
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
    let provider = FixtureConfig {
        multipart: call.multipart,
    };
    let image = call.image.file();
    let params = litellm_params(call, api_base);
    let edit = edit_call(call, &image, &params);
    let deps = ImageEditDeps {
        config: &provider,
        source: &origin,
        supplied: None,
    };
    let mut fixture = Fixture::new()?;
    let result = fixture.run(call.mode.client_kind(), &deps, &edit).await;
    let answer = outcome(result, fixture.logged())?;
    Ok(SdkResult::ImageEditHttp {
        value: Box::new(answer),
    })
}

fn litellm_params(call: &Call, api_base: &str) -> ImageParams {
    let mut params = ImageParams::new();
    params.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    if let Some(api_key) = &call.api_key {
        params.insert(String::from("api_key"), Json::Text(api_key.clone()));
    }
    if let Some(ssl_verify) = call.ssl_verify {
        params.insert(String::from("ssl_verify"), Json::Bool(ssl_verify));
    }
    params
}

fn edit_call<'call>(
    call: &'call Call,
    image: &'call ImageFile,
    params: &'call ImageParams,
) -> ImageEditCall<'call> {
    ImageEditCall {
        model: &call.model,
        prompt: call.prompt.as_deref(),
        image: Some(image),
        optional_params: &call.optional_params,
        litellm_params: params,
        extra_headers: call.extra_headers.as_ref(),
        timeout: Duration::from_secs(call.timeout_seconds),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    result: Result<ImageEditOutcome, ImageEditHttpError<FixtureError>>,
    logged: Option<LoggedCall>,
) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(edit) => Ok(Outcome {
            result: Some(edit.response),
            provider_headers: Some(edit.provider_headers.headers.into_iter().collect()),
            failure: None,
            logged,
        }),
        Err(ImageEditHttpError::Provider(error)) => Ok(Outcome {
            result: None,
            provider_headers: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error
                    .headers
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, value)| (name, Json::Text(value)))
                    .collect(),
            }),
            logged,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

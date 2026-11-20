use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use secrecy::SecretString;
use url::Url;

use super::error::{ClientFailure, ConfigFailure, ImageGenerationError};
use super::{ClientKind, ClientSource, ImageGenerationCall, ImageGenerationDeps, Selected};
use crate::api::image_contracts::generation::{BaseImageGenerationConfig, ImageGenerationRequest};
use crate::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest};

// A supplied client answers only the mode it was built for; the other mode takes the shared client
pub(super) fn select<'borrow, 'deps, C, S>(
    deps: &'borrow ImageGenerationDeps<'deps, C, S>,
    mode: ClientKind,
    call: &ImageGenerationCall<'_>,
) -> Result<Selected<'borrow, S::Client>, ImageGenerationError>
where
    'deps: 'borrow,
    C: BaseImageGenerationConfig,
    S: ClientSource,
{
    let ssl_verify = match call.litellm_params.get("ssl_verify") {
        None | Some(Json::Null) => None,
        Some(Json::Bool(setting)) => Some(*setting),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ImageGenerationError::FieldType {
                field: "litellm_params.ssl_verify",
                expected: "a boolean",
            });
        }
    };
    match &deps.supplied {
        Some(supplied) if supplied.kind == mode => Ok(Selected::Supplied(supplied.transport)),
        Some(_) | None => deps
            .source
            .client(ssl_verify)
            .map(Selected::Created)
            .map_err(|source| {
                ImageGenerationError::Client(ClientFailure {
                    source: Box::new(source),
                })
            }),
    }
}

pub(super) fn validated_headers<C>(
    config: &C,
    call: &ImageGenerationCall<'_>,
) -> Result<ImageHeaders, ImageGenerationError>
where
    C: BaseImageGenerationConfig,
{
    let caller = caller_headers(call.optional_params)?;
    let explicit = call.api_key.map(|key| SecretString::from(key.to_owned()));
    let mut headers = config.validate_environment(ImageEnvironment {
        headers: &caller,
        model: call.model,
        messages: &[],
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
        api_key: explicit.as_ref(),
        api_base: None,
    });
    if let Some(extra) = call.extra_headers {
        headers.extend(extra.clone());
    }
    Ok(headers)
}

fn caller_headers(optional_params: &ImageParams) -> Result<ImageHeaders, ImageGenerationError> {
    match optional_params.get("extra_headers") {
        None | Some(Json::Null) => Ok(ImageHeaders::new()),
        Some(Json::Object(fields)) => fields
            .iter()
            .map(|(name, value)| match value {
                Json::Text(text) => Ok((name.clone(), text.clone())),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(ImageGenerationError::HeaderType { name: name.clone() })
                }
            })
            .collect(),
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(ImageGenerationError::HeadersType)
        }
    }
}

pub(super) fn endpoint<C>(
    config: &C,
    call: &ImageGenerationCall<'_>,
) -> Result<(String, Url), ImageGenerationError>
where
    C: BaseImageGenerationConfig,
{
    let params = call.litellm_params;
    let stored = match params.get("api_key") {
        None | Some(Json::Null) => None,
        Some(Json::Text(key)) => Some(SecretString::from(key.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ImageGenerationError::FieldType {
                field: "litellm_params.api_key",
                expected: "a string",
            });
        }
    };
    let given = match params.get("api_base") {
        None | Some(Json::Null) => None,
        Some(Json::Text(base)) => Some(base.as_str()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ImageGenerationError::FieldType {
                field: "litellm_params.api_base",
                expected: "a string",
            });
        }
    };
    let api_base = config.get_complete_url(ImageUrlRequest {
        api_base: given,
        api_key: stored.as_ref(),
        model: call.model,
        optional_params: call.optional_params,
        litellm_params: params,
        stream: None,
    });
    let url = Url::parse(&api_base).map_err(|source| ImageGenerationError::RequestUrl {
        url: api_base.clone(),
        source,
    })?;
    Ok((api_base, url))
}

pub(super) fn transform<C>(
    config: &C,
    call: &ImageGenerationCall<'_>,
    headers: &ImageHeaders,
) -> Result<ImageParams, ImageGenerationError>
where
    C: BaseImageGenerationConfig,
{
    config
        .transform_image_generation_request(ImageGenerationRequest {
            model: call.model,
            prompt: call.prompt,
            optional_params: call.optional_params,
            litellm_params: call.litellm_params,
            headers,
        })
        .map_err(|source| {
            ImageGenerationError::Config(ConfigFailure {
                source: Box::new(source),
            })
        })
}

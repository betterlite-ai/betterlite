use percent_encoding::percent_decode_str;
use serde::Serialize;

use betterlite_prompts::media::base64_string::get_base64_str;

use crate::image_models::params::{Field, Params};
use crate::marengo_three::build_marengo_3_request;
use crate::marengo_three::is_marengo_3_model;
use crate::marengo_three::params::RawParams;
use crate::marengo_three::request::Marengo3Request;

use super::error::MarengoEmbeddingsError;
use super::params::resolved_input_type;

const CORE_FIELDS: [&str; 5] = [
    "inputType",
    "input_type",
    "inputText",
    "mediaSource",
    "bucketOwner",
];
const S3_PREFIX: &str = "s3://";
const DATA_PREFIX: &str = "data:";
const ASYNC_INVOKE_PREFIX: &str = "async_invoke/";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MarengoModelInput {
    Flat(Params),
    Nested(Marengo3Request),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3OutputDataConfig {
    pub s3_uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDataConfig {
    pub s3_output_data_config: S3OutputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncInvokeRequest {
    pub model_id: String,
    pub model_input: MarengoModelInput,
    pub output_data_config: OutputDataConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MarengoRequest {
    Wrapped(AsyncInvokeRequest),
    Direct(MarengoModelInput),
}

#[derive(Debug)]
pub struct TransformRequest<'a> {
    pub model: Option<&'a str>,
    pub input: &'a str,
    pub inference_params: &'a Params,
    pub async_invoke_route: bool,
    pub model_id: Option<&'a str>,
    pub output_s3_uri: Option<&'a str>,
    pub drop_params: bool,
}

pub fn transform_request(
    request: TransformRequest<'_>,
) -> Result<MarengoRequest, MarengoEmbeddingsError> {
    let input_type = resolved_input_type(request.inference_params);
    if matches!(input_type.as_str(), Some("video" | "audio")) && !request.async_invoke_route {
        return Err(MarengoEmbeddingsError::AsyncRouteRequired {
            input_type: input_type.python_str(),
        });
    }
    let model_input = if is_marengo_3_model(request.model) {
        MarengoModelInput::Nested(build_marengo_3_request(
            request.input,
            &marengo_3_params(request.inference_params)?,
            request.drop_params,
        )?)
    } else {
        MarengoModelInput::Flat(flat_body(
            request.input,
            &input_type,
            request.inference_params,
        ))
    };
    if request.async_invoke_route
        && let Some(model_id) = request.model_id.filter(|value| !value.is_empty())
    {
        return Ok(MarengoRequest::Wrapped(wrap_async_invoke_request(
            model_input,
            model_id,
            request.output_s3_uri,
        )?));
    }
    Ok(MarengoRequest::Direct(model_input))
}

fn flat_body(input: &str, input_type: &Field, inference_params: &Params) -> Params {
    let mut body = Params::new();
    body.insert("inputType".to_owned(), input_type.clone());
    match input_type.as_str() {
        Some("text") => {
            body.insert("inputText".to_owned(), Field::Text(input.to_owned()));
            if !inference_params.contains_key("textTruncate") {
                body.insert("textTruncate".to_owned(), Field::Text("end".to_owned()));
            }
        }
        Some("image" | "video" | "audio") => {
            body.insert(
                "mediaSource".to_owned(),
                Field::Mapping(media_source(input, inference_params)),
            );
        }
        _ => {}
    }
    for (key, value) in inference_params {
        if !CORE_FIELDS.contains(&key.as_str()) {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

fn media_source(input: &str, inference_params: &Params) -> Params {
    let mut source = Params::new();
    if input.starts_with(S3_PREFIX) {
        let mut location = Params::new();
        location.insert("uri".to_owned(), Field::Text(input.to_owned()));
        if let Some(owner) = inference_params
            .get("bucketOwner")
            .filter(|value| value.truthy())
        {
            location.insert("bucketOwner".to_owned(), owner.clone());
        }
        source.insert("s3Location".to_owned(), Field::Mapping(location));
    } else {
        source.insert(
            "base64String".to_owned(),
            Field::Text(encoded_media(input).to_owned()),
        );
    }
    source
}

fn encoded_media(input: &str) -> &str {
    let Some(data) = input.strip_prefix(DATA_PREFIX) else {
        return get_base64_str(input);
    };
    data.split_once(',').map_or(input, |(_head, tail)| tail)
}

fn marengo_3_params(inference_params: &Params) -> Result<RawParams, MarengoEmbeddingsError> {
    let value = serde_json::to_value(inference_params)
        .map_err(|source| MarengoEmbeddingsError::Parameters { source })?;
    serde_json::from_value(value).map_err(|source| MarengoEmbeddingsError::Parameters { source })
}

fn wrap_async_invoke_request(
    model_input: MarengoModelInput,
    model_id: &str,
    output_s3_uri: Option<&str>,
) -> Result<AsyncInvokeRequest, MarengoEmbeddingsError> {
    let unquoted = percent_decode_str(model_id).decode_utf8_lossy();
    let cleaned = if unquoted.starts_with(ASYNC_INVOKE_PREFIX) {
        unquoted.replace(ASYNC_INVOKE_PREFIX, "")
    } else {
        unquoted.into_owned()
    };
    let destination = output_s3_uri.ok_or(MarengoEmbeddingsError::EmptyOutputS3Uri)?;
    if destination.trim().is_empty() {
        return Err(MarengoEmbeddingsError::EmptyOutputS3Uri);
    }
    Ok(AsyncInvokeRequest {
        model_id: cleaned,
        model_input,
        output_data_config: OutputDataConfig {
            s3_output_data_config: S3OutputDataConfig {
                s3_uri: destination.to_owned(),
            },
        },
    })
}

use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::nova_embeddings::data_url::parse_nova_data_url;
use betterlite_bedrock::nova_embeddings::error::NovaEmbeddingsError;
use betterlite_bedrock::nova_embeddings::request::{
    SUPPORTED_OPENAI_PARAMS, map_openai_params, transform_request,
};
use betterlite_bedrock::nova_embeddings::response::{
    transform_invoke_response, transform_response,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod calls;

use calls::{
    DataUrlCall, DefaultPurposeCall, InvocationArnCall, TransformRequestCall, TransformResponseCall,
};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    NovaDefaultPurposeDimensions(DefaultPurposeCall),
    NovaTextImageAudioVideo(TransformRequestCall),
    NovaDataUrlJpg(DataUrlCall),
    NovaExplicitModality(TransformRequestCall),
    NovaAsyncOutputS3Required(TransformRequestCall),
    NovaResultIndexTokenEstimate(TransformResponseCall),
    NovaInvocationArn(InvocationArnCall),
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    NovaEmbeddings(#[from] NovaEmbeddingsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::NovaDefaultPurposeDimensions(call) => nova_default_dimensions(call),
            Input::NovaTextImageAudioVideo(call)
            | Input::NovaExplicitModality(call)
            | Input::NovaAsyncOutputS3Required(call) => run_transform_request(call),
            Input::NovaDataUrlJpg(call) => nova_data_url_jpg(call),
            Input::NovaResultIndexTokenEstimate(call) => nova_result_tokens(call),
            Input::NovaInvocationArn(call) => Ok(nova_invocation_arn(call)),
        };
        Ok(output?)
    }
}

fn nova_default_dimensions(call: &DefaultPurposeCall) -> Result<Value, Failure> {
    let non_default = map_to_params(&call.non_default)?;
    let optional_params = map_to_params(&call.optional_params)?;
    let mapped = map_openai_params(&non_default, optional_params);
    let inference_params = map_to_params(&call.inference_params)?;
    let request = transform_request(
        &call.input,
        &inference_params,
        call.async_invoke_route,
        call.model_id.as_deref(),
        call.output_s3_uri.as_deref(),
    )?;
    Ok(json!({
        "supported_params": SUPPORTED_OPENAI_PARAMS,
        "mapped": Value::Object(params_to_map(&mapped)?),
        "request": Value::Object(params_to_map(&request)?),
    }))
}

fn run_transform_request(call: &TransformRequestCall) -> Result<Value, Failure> {
    let inference_params = map_to_params(&call.inference_params)?;
    let request = transform_request(
        &call.input,
        &inference_params,
        call.async_invoke_route,
        call.model_id.as_deref(),
        call.output_s3_uri.as_deref(),
    )?;
    Ok(Value::Object(params_to_map(&request)?))
}

fn nova_data_url_jpg(call: &DataUrlCall) -> Result<Value, Failure> {
    let parsed = parse_nova_data_url(&call.data_url)?;
    let request = transform_request(&call.data_url, &Params::new(), false, None, None)?;
    Ok(json!({
        "media_type": parsed.media_type,
        "data": parsed.data,
        "request": Value::Object(params_to_map(&request)?),
    }))
}

fn nova_result_tokens(call: &TransformResponseCall) -> Result<Value, Failure> {
    let response_list = call
        .response_list
        .iter()
        .map(map_to_params)
        .collect::<Result<Vec<_>, Failure>>()?;
    let batch_data = call
        .batch_data
        .as_ref()
        .map(|batch| {
            batch
                .iter()
                .map(map_to_params)
                .collect::<Result<Vec<_>, Failure>>()
        })
        .transpose()?;
    let result = transform_response(&response_list, "nova-model", batch_data.as_deref())?;
    embedding_response_json(&result)
}

fn nova_invocation_arn(call: &InvocationArnCall) -> Value {
    let response = map_to_params(&call.response).unwrap_or_default();
    let result = transform_invoke_response(&response, "nova-model");
    embedding_response_json(&result).unwrap_or(Value::Null)
}

fn embedding_response_json(
    result: &betterlite_bedrock::nova_embeddings::response::NovaEmbeddingResponse,
) -> Result<Value, Failure> {
    let data = result
        .data
        .iter()
        .map(|record| serde_json::to_value(&record.embedding))
        .collect::<Result<Vec<_>, _>>()?;
    let prompt_tokens_details = result
        .usage
        .prompt_tokens_details
        .as_ref()
        .map(|details| json!({"image_count": details.image_count}));
    Ok(json!({
        "data": data,
        "model": result.model,
        "usage": {
            "prompt_tokens": result.usage.prompt_tokens,
            "total_tokens": result.usage.total_tokens,
            "prompt_tokens_details": prompt_tokens_details,
        },
        "invocation_arn": result.invocation_arn,
    }))
}

fn map_to_params(map: &Map<String, Value>) -> Result<Params, Failure> {
    let value = Value::Object(map.clone());
    let params: Params = serde_json::from_value(value)?;
    Ok(params)
}

fn params_to_map(params: &Params) -> Result<Map<String, Value>, Failure> {
    let mut map = Map::new();
    for (key, field) in params {
        map.insert(key.clone(), serde_json::to_value(field)?);
    }
    Ok(map)
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::NovaEmbeddings(
                NovaEmbeddingsError::InvalidDataUrl { .. }
                | NovaEmbeddingsError::MissingComma { .. }
                | NovaEmbeddingsError::OutputS3UriRequired,
            ) => "ValueError",
            Failure::NovaEmbeddings(NovaEmbeddingsError::MalformedField { .. }) => "TypeError",
            Failure::NovaEmbeddings(NovaEmbeddingsError::TokenOverflow) => "OverflowError",
            Failure::Json(_) => "KeyError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

mod tests;

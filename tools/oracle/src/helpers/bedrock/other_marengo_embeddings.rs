use betterlite_bedrock::image_models::params::{Field, Params};
use betterlite_bedrock::marengo_embeddings::error::MarengoEmbeddingsError;
use betterlite_bedrock::marengo_embeddings::params::{
    drop_params_enabled, supported_openai_params,
};
use betterlite_bedrock::marengo_embeddings::request::{TransformRequest, transform_request};
use betterlite_bedrock::marengo_embeddings::response::{
    MarengoBilledRequest, MarengoEmbeddingResponse, MarengoInvokeResponse,
    transform_async_invoke_response, transform_response,
};
use betterlite_bedrock::marengo_three::error::Marengo3Error;
use betterlite_context::non_default_params::mapper::DropPolicy;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Call {
    model: String,
    input: Vec<String>,
    inference: Params,
    replies: Vec<Field>,
    async_invoke_route: bool,
    model_id: Option<String>,
    output_s3_uri: Option<String>,
    drop_params: bool,
    batch: Vec<Field>,
    global_drop_params: bool,
    deployment_drop_params: Option<Field>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MarengoTextInput(Call),
    MarengoS3VideoOwner(Call),
    MarengoImageInput(Call),
    MarengoAsyncOutputWrapper(Call),
    MarengoSegmentsVectorList(Call),
    MarengoBilledUnits(Call),
    DropUnsupportedParam(Call),
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Embeddings(#[from] MarengoEmbeddingsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the response for an input is missing")]
    MissingReply,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DropUnsupportedParam(call) => Ok(json!({
                "supported": supported_openai_params(Some(&call.model)),
                "enabled": drop_params_enabled(policy(call)),
            })),
            Input::MarengoTextInput(call)
            | Input::MarengoS3VideoOwner(call)
            | Input::MarengoImageInput(call)
            | Input::MarengoAsyncOutputWrapper(call)
            | Input::MarengoSegmentsVectorList(call)
            | Input::MarengoBilledUnits(call) => {
                let requests = transform_all(call)?;
                let response = response_value(call, &requests)?;
                Ok(json!({
                    "supported": supported_openai_params(Some(&call.model)),
                    "requests": requests,
                    "response": response,
                }))
            }
        }
    }
}

fn policy(call: &Call) -> DropPolicy {
    DropPolicy {
        drop_params: match &call.deployment_drop_params {
            Some(Field::Bool(flag)) => Some(*flag),
            Some(
                Field::Null
                | Field::Int(_)
                | Field::Float(_)
                | Field::Text(_)
                | Field::List(_)
                | Field::Mapping(_),
            )
            | None => None,
        },
        litellm_drop_params: call.global_drop_params,
    }
}

fn transform_all(call: &Call) -> Result<Vec<Value>, Failure> {
    let mut requests = Vec::new();
    for input in &call.input {
        let request = transform_request(TransformRequest {
            model: Some(&call.model),
            input,
            inference_params: &call.inference,
            async_invoke_route: call.async_invoke_route,
            model_id: call.model_id.as_deref(),
            output_s3_uri: call.output_s3_uri.as_deref(),
            drop_params: call.drop_params,
        })?;
        requests.push(serde_json::to_value(&request)?);
    }
    Ok(requests)
}

fn response_value(call: &Call, requests: &[Value]) -> Result<Value, Failure> {
    if call.replies.len() < call.input.len() {
        return Err(Failure::MissingReply);
    }
    let first = call.replies.first();
    let response: MarengoEmbeddingResponse = if call.async_invoke_route
        && call.input.len() == 1
        && first.is_some_and(has_invocation_arn)
    {
        let Some(reply) = first else {
            return Err(Failure::MissingReply);
        };
        let parsed: Params = serde_json::from_value(serde_json::to_value(reply)?)?;
        transform_async_invoke_response(&parsed, &call.model)
    } else {
        let responses = call
            .replies
            .iter()
            .take(call.input.len())
            .map(|reply| Ok(serde_json::from_value::<MarengoInvokeResponse>(reply_value(reply)?)?))
            .collect::<Result<Vec<_>, Failure>>()?;
        let batch = batch_value(call, requests)?;
        transform_response(&responses, &call.model, Some(&batch))?
    };
    Ok(serde_json::to_value(&response)?)
}

fn batch_value(call: &Call, requests: &[Value]) -> Result<Vec<MarengoBilledRequest>, Failure> {
    let entries: Vec<Value> = if call.batch.is_empty() {
        requests.to_vec()
    } else {
        call.batch
            .iter()
            .map(|entry| Ok(serde_json::to_value(entry)?))
            .collect::<Result<Vec<_>, Failure>>()?
    };
    entries
        .into_iter()
        .map(|entry| serde_json::from_value(entry).map_err(Failure::from))
        .collect()
}

fn reply_value(reply: &Field) -> Result<Value, Failure> {
    Ok(serde_json::to_value(reply)?)
}

fn has_invocation_arn(reply: &Field) -> bool {
    matches!(reply, Field::Mapping(entries) if entries.contains_key("invocationArn"))
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let (class, status) = match &failure {
            Failure::Embeddings(MarengoEmbeddingsError::Marengo3(Marengo3Error::Provider {
                status,
                ..
            })) => ("BedrockError", Some(*status)),
            Failure::Embeddings(MarengoEmbeddingsError::Marengo3(Marengo3Error::Header(_))) => {
                ("HeaderError", None)
            }
            Failure::Embeddings(
                MarengoEmbeddingsError::AsyncRouteRequired { .. }
                | MarengoEmbeddingsError::EmptyOutputS3Uri
                | MarengoEmbeddingsError::UsageOverflow
                | MarengoEmbeddingsError::Parameters { .. },
            )
            | Failure::Json(_)
            | Failure::MissingReply => ("ValueError", None),
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

use betterlite_bedrock::image_models::params::{Field, Params};
use betterlite_bedrock::marengo_embeddings::error::MarengoEmbeddingsError;
use betterlite_bedrock::marengo_embeddings::params::{
    drop_params_enabled, supported_openai_params,
};
use betterlite_bedrock::marengo_embeddings::request::{TransformRequest, transform_request};
use betterlite_bedrock::marengo_embeddings::response::{
    MarengoBilledRequest, MarengoEmbeddingResponse, MarengoInvokeResponse,
    transform_async_response, transform_response,
};
use betterlite_bedrock::marengo_three::error::Marengo3Error;
use betterlite_context::non_default_params::mapper::DropPolicy;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use thiserror::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Call {
    model: String,
    input: Vec<String>,
    inference: Params,
    replies: Vec<Field>,
    async_invoke_route: Option<bool>,
    model_id: Option<String>,
    output_s3_uri: Option<String>,
    drop_params: Option<bool>,
    batch: Vec<Field>,
    global_drop_params: Option<bool>,
    deployment_drop_params: Option<Field>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "marengo_text_input")]
    MarengoText(Call),
    MarengoS3VideoOwner(Call),
    #[serde(rename = "marengo_image_input")]
    MarengoImage(Call),
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
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DropUnsupportedParam(call) => Ok(json!({
                "supported": supported_openai_params(Some(&call.model)),
                "enabled": drop_params_enabled(policy(call)),
            })),
            Input::MarengoText(call)
            | Input::MarengoS3VideoOwner(call)
            | Input::MarengoImage(call)
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
        litellm_drop_params: call.global_drop_params.unwrap_or(false),
    }
}

fn transform_all(call: &Call) -> Result<Vec<Value>, Failure> {
    let mut requests = Vec::new();
    for input in &call.input {
        let request = transform_request(TransformRequest {
            model: Some(&call.model),
            input,
            inference_params: &call.inference,
            async_invoke_route: call.async_invoke_route.unwrap_or(false),
            model_id: call.model_id.as_deref(),
            output_s3_uri: call.output_s3_uri.as_deref(),
            drop_params: call.drop_params.unwrap_or(false),
        })?;
        requests.push(serde_json::to_value(request)?);
    }
    Ok(requests)
}

fn async_reply(call: &Call) -> Option<&Field> {
    call.replies.first().filter(|reply| {
        call.async_invoke_route.unwrap_or(false)
            && call.input.len() == 1
            && call.replies.len() == 1
            && matches!(*reply, Field::Mapping(entries) if entries.contains_key("invocationArn"))
    })
}

fn parse_all<T>(values: &[Value]) -> Result<Vec<T>, Failure>
where
    T: DeserializeOwned,
{
    values
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Failure::from)
}

fn response_value(call: &Call, requests: &[Value]) -> Result<Value, Failure> {
    let response: MarengoEmbeddingResponse = if let Some(reply) = async_reply(call) {
        let reply_json = serde_json::to_value(reply)?;
        let parsed: Params = serde_json::from_value(reply_json)?;
        transform_async_response(&parsed, &call.model)
    } else {
        let replies = call.replies.iter().take(call.input.len());
        let encoded: Vec<Value> = replies
            .map(serde_json::to_value)
            .collect::<Result<_, serde_json::Error>>()?;
        let responses: Vec<MarengoInvokeResponse> = parse_all(&encoded)?;
        let batch = batch_value(call, requests)?;
        transform_response(&responses, &call.model, Some(&batch))?
    };
    Ok(serde_json::to_value(response)?)
}

fn batch_value(call: &Call, requests: &[Value]) -> Result<Vec<MarengoBilledRequest>, Failure> {
    if call.batch.is_empty() {
        return parse_all(requests);
    }
    let encoded: Vec<Value> = call
        .batch
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, serde_json::Error>>()?;
    parse_all(&encoded)
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
                | MarengoEmbeddingsError::Nova(_)
                | MarengoEmbeddingsError::UsageOverflow
                | MarengoEmbeddingsError::VectorValue
                | MarengoEmbeddingsError::Parameters { .. },
            )
            | Failure::Json(_) => ("ValueError", None),
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

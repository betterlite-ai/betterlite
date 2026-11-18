use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use super::convert::{encode_json, parse_list, parse_one, parse_optional};
use super::error::EmbeddingIoError;
use crate::aws_endpoints::provider::BedrockEmbeddingProvider;
use crate::image_models::params::{Field, Params};
use crate::marengo_embeddings::response::{
    self as marengo, MarengoBilledRequest, MarengoEmbeddingResponse, MarengoInvokeResponse,
};
use crate::nova_embeddings::response::{self as nova, NovaEmbeddingResponse};
use crate::titan_embeddings::response::{self as titan, TitanEmbeddingResponse};
use crate::titan_embeddings::{multimodal, v2};

const TITAN_MULTIMODAL_MODEL: &str = "amazon.titan-embed-image-v1";
const TITAN_G1_MODEL: &str = "amazon.titan-embed-text-v1";
const TITAN_G1_LEGACY_MODEL: &str = "amazon.titan-embed-g1-text-02";
const TITAN_V2_MODEL: &str = "amazon.titan-embed-text-v2:0";
const RESPONSE_OBJECT: &str = "list";
const EMBEDDING_OBJECT: &str = "embedding";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmbeddingReply {
    pub object: &'static str,
    pub data: Vec<EmbeddingRecord>,
    pub model: String,
    pub usage: EmbeddingUsage,
    pub invocation_arn: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_tokens_details: Option<PromptTokenCounts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PromptTokenCounts {
    pub image_count: Option<u64>,
    pub query_count: Option<u64>,
}

pub fn transform_response(
    response_list: &[Object],
    model: &str,
    provider: BedrockEmbeddingProvider,
    is_async_invoke: bool,
    batch_data: Option<&[Object]>,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    if let Some(reply) = response_list.first()
        && is_async_invoke
        && response_list.len() == 1
        && reply.contains_key("invocationArn")
    {
        return async_reply(reply, model, provider);
    }
    regular_reply(response_list, model, provider, batch_data)
}

// only a single reply that names the ARN enters the async branch; a null ARN is still an ARN
fn async_reply(
    reply: &Object,
    model: &str,
    provider: BedrockEmbeddingProvider,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    match provider {
        BedrockEmbeddingProvider::Twelvelabs => {
            let params = parse_one::<Params>(reply)?;
            marengo_embedding_reply(marengo::transform_async_response(&params, model))
        }
        BedrockEmbeddingProvider::Nova => {
            let params = parse_one::<Params>(reply)?;
            nova_embedding_reply(nova::transform_invoke_response(&params, model))
        }
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => {
            Ok(placeholder(reply, model))
        }
    }
}

// the exact titan models are chosen before the provider, which only sees twelvelabs and nova
fn regular_reply(
    response_list: &[Object],
    model: &str,
    provider: BedrockEmbeddingProvider,
    batch_data: Option<&[Object]>,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    if model == TITAN_MULTIMODAL_MODEL {
        let response = multimodal::transform_response(response_list, model, batch_data)?;
        return Ok(titan_embedding_reply(response));
    }
    if model == TITAN_G1_MODEL || model == TITAN_G1_LEGACY_MODEL {
        let response = titan::transform_response(response_list, model, titan::plain_embedding)?;
        return Ok(titan_embedding_reply(response));
    }
    if model == TITAN_V2_MODEL {
        let response = titan::transform_response(response_list, model, v2::by_type_embedding)?;
        return Ok(titan_embedding_reply(response));
    }
    match provider {
        BedrockEmbeddingProvider::Twelvelabs => {
            let replies = parse_list::<MarengoInvokeResponse>(response_list)?;
            let batch = parse_optional::<MarengoBilledRequest>(batch_data)?;
            let response = marengo::transform_response(&replies, model, batch.as_deref())?;
            marengo_embedding_reply(response)
        }
        BedrockEmbeddingProvider::Nova => {
            let replies = parse_list::<Params>(response_list)?;
            let batch = parse_optional::<Params>(batch_data)?;
            let response = nova::transform_response(&replies, model, batch.as_deref())?;
            nova_embedding_reply(response)
        }
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => {
            Err(EmbeddingIoError::Unmapped {
                model: model.to_owned(),
            })
        }
    }
}

// other providers get one placeholder record that carries the ARN beside the reply
fn placeholder(reply: &Object, model: &str) -> EmbeddingReply {
    EmbeddingReply {
        object: RESPONSE_OBJECT,
        data: vec![EmbeddingRecord {
            object: EMBEDDING_OBJECT.to_owned(),
            embedding: EmbeddingVector::Floats(Vec::new()),
            index: 0,
        }],
        model: model.to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_tokens_details: None,
        },
        invocation_arn: reply.get("invocationArn").cloned().unwrap_or(Json::Null),
    }
}

fn titan_embedding_reply(response: TitanEmbeddingResponse) -> EmbeddingReply {
    let details = response
        .usage
        .prompt_tokens_details
        .map(|counts| PromptTokenCounts {
            image_count: Some(u64::from(counts.image_count)),
            query_count: None,
        });
    EmbeddingReply {
        object: response.object,
        data: response.data,
        model: response.model,
        usage: EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: details,
        },
        invocation_arn: Json::Null,
    }
}

fn nova_embedding_reply(
    response: NovaEmbeddingResponse,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    let mut data = Vec::with_capacity(response.data.len());
    for (position, record) in response.data.into_iter().enumerate() {
        let index = u32::try_from(position).map_err(|_overflow| EmbeddingIoError::IndexOverflow)?;
        data.push(EmbeddingRecord {
            object: EMBEDDING_OBJECT.to_owned(),
            embedding: embedding_vector(&record.embedding)?,
            index,
        });
    }
    let details = response
        .usage
        .prompt_tokens_details
        .map(|counts| PromptTokenCounts {
            image_count: Some(u64::from(counts.image_count)),
            query_count: None,
        });
    let invocation_arn = match &response.invocation_arn {
        Some(field) => encode_json(field)?,
        None => Json::Null,
    };
    Ok(EmbeddingReply {
        object: RESPONSE_OBJECT,
        data,
        model: response.model,
        usage: EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: 0,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: details,
        },
        invocation_arn,
    })
}

fn marengo_embedding_reply(
    response: MarengoEmbeddingResponse,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    let mut data = Vec::with_capacity(response.data.len());
    for record in response.data {
        let index =
            u32::try_from(record.index).map_err(|_overflow| EmbeddingIoError::IndexOverflow)?;
        let mut numbers = Vec::with_capacity(record.embedding.len());
        for value in record.embedding {
            let number = Number::from_f64(value).ok_or(EmbeddingIoError::Unrepresentable)?;
            numbers.push(number);
        }
        data.push(EmbeddingRecord {
            object: EMBEDDING_OBJECT.to_owned(),
            embedding: EmbeddingVector::Floats(numbers),
            index,
        });
    }
    let details = response
        .usage
        .prompt_tokens_details
        .map(|counts| PromptTokenCounts {
            image_count: counts.image_count,
            query_count: counts.query_count,
        });
    let invocation_arn = match &response.invocation_arn {
        Some(field) => encode_json(field)?,
        None => Json::Null,
    };
    Ok(EmbeddingReply {
        object: RESPONSE_OBJECT,
        data,
        model: response.model,
        usage: EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: details,
        },
        invocation_arn,
    })
}

// Nova declares embedding as list | str, and the shared record carries only those two shapes
fn embedding_vector(field: &Field) -> Result<EmbeddingVector, EmbeddingIoError> {
    match field {
        Field::Text(encoded) => Ok(EmbeddingVector::Base64(encoded.clone())),
        Field::List(items) => {
            let mut numbers = Vec::with_capacity(items.len());
            for item in items {
                numbers.push(item_number(item)?);
            }
            Ok(EmbeddingVector::Floats(numbers))
        }
        Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::Mapping(_) => {
            Err(EmbeddingIoError::MalformedEmbedding)
        }
    }
}

fn item_number(field: &Field) -> Result<Number, EmbeddingIoError> {
    match field {
        Field::Int(value) => Ok(Number::from(*value)),
        Field::Float(value) => Number::from_f64(*value).ok_or(EmbeddingIoError::Unrepresentable),
        Field::Null | Field::Bool(_) | Field::Text(_) | Field::List(_) | Field::Mapping(_) => {
            Err(EmbeddingIoError::NotANumber)
        }
    }
}

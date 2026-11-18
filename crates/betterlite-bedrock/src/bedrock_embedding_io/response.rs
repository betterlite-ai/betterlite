use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use super::convert::{encode_json, parse_list, parse_one};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
        return match async_response(reply, model, provider)? {
            Some(response) => provider_embedding_reply(response),
            None => Ok(EmbeddingReply {
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
            }),
        };
    }
    regular_reply(response_list, model, provider, batch_data)
}

// only a single reply that names the ARN enters the async branch; a null ARN is still an ARN
fn async_response(
    reply: &Object,
    model: &str,
    provider: BedrockEmbeddingProvider,
) -> Result<Option<ProviderResponse>, EmbeddingIoError> {
    match provider {
        BedrockEmbeddingProvider::Twelvelabs => Ok(Some(ProviderResponse::Marengo(
            marengo::transform_async_response(&parse_one::<Params>(reply)?, model),
        ))),
        BedrockEmbeddingProvider::Nova => Ok(Some(ProviderResponse::Nova(
            nova::transform_invoke_response(&parse_one::<Params>(reply)?, model),
        ))),
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => Ok(None),
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
            let batch = batch_data
                .map(parse_list::<MarengoBilledRequest>)
                .transpose()?;
            let response = marengo::transform_response(&replies, model, batch.as_deref())?;
            provider_embedding_reply(ProviderResponse::Marengo(response))
        }
        BedrockEmbeddingProvider::Nova => {
            let replies = parse_list::<Params>(response_list)?;
            let batch = batch_data.map(parse_list::<Params>).transpose()?;
            let response = nova::transform_response(&replies, model, batch.as_deref())?;
            provider_embedding_reply(ProviderResponse::Nova(response))
        }
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => {
            Err(EmbeddingIoError::Unmapped {
                model: model.to_owned(),
            })
        }
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
    finish_reply(
        response.object,
        response.model,
        response.data,
        EmbeddingUsage {
            prompt_tokens: response.usage.prompt_tokens,
            completion_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
            prompt_tokens_details: details,
        },
        Json::Null,
    )
}

enum ProviderResponse {
    Nova(NovaEmbeddingResponse),
    Marengo(MarengoEmbeddingResponse),
}

struct ProviderUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_tokens_details: Option<PromptTokenCounts>,
}

fn provider_embedding_reply(
    provider_response: ProviderResponse,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    match provider_response {
        ProviderResponse::Nova(response) => {
            let records = response
                .data
                .into_iter()
                .enumerate()
                .map(|(position, record)| {
                    let index = u32::try_from(position)
                        .map_err(|_overflow| EmbeddingIoError::IndexOverflow)?;
                    Ok((index, embedding_vector(&record.embedding)?))
                });
            provider_reply(
                response.model,
                records,
                nova_usage(response.usage),
                response.invocation_arn.as_ref(),
            )
        }
        ProviderResponse::Marengo(response) => {
            let records = response.data.into_iter().map(marengo_record);
            provider_reply(
                response.model,
                records,
                marengo_usage(response.usage),
                response.invocation_arn.as_ref(),
            )
        }
    }
}

fn marengo_record(
    record: marengo::EmbeddingRecord,
) -> Result<(u32, EmbeddingVector), EmbeddingIoError> {
    let index = u32::try_from(record.index).map_err(|_overflow| EmbeddingIoError::IndexOverflow)?;
    let numbers = record
        .embedding
        .into_iter()
        .map(|value| Number::from_f64(value).ok_or(EmbeddingIoError::Unrepresentable))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((index, EmbeddingVector::Floats(numbers)))
}

fn nova_usage(usage: nova::NovaUsage) -> ProviderUsage {
    let details = usage.prompt_tokens_details.map(|counts| PromptTokenCounts {
        image_count: Some(u64::from(counts.image_count)),
        query_count: None,
    });
    provider_usage(usage.prompt_tokens, 0, usage.total_tokens, details)
}

fn marengo_usage(usage: marengo::MarengoUsage) -> ProviderUsage {
    let details = usage.prompt_tokens_details.map(|counts| PromptTokenCounts {
        image_count: counts.image_count,
        query_count: counts.query_count,
    });
    provider_usage(
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
        details,
    )
}

fn provider_usage(
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_tokens_details: Option<PromptTokenCounts>,
) -> ProviderUsage {
    ProviderUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details,
    }
}

fn embedding_records<I>(records: I) -> Result<Vec<EmbeddingRecord>, EmbeddingIoError>
where
    I: IntoIterator<Item = Result<(u32, EmbeddingVector), EmbeddingIoError>>,
{
    records
        .into_iter()
        .map(|record| {
            let (index, embedding) = record?;
            Ok(EmbeddingRecord {
                object: EMBEDDING_OBJECT.to_owned(),
                embedding,
                index,
            })
        })
        .collect()
}

fn provider_reply<I>(
    model: String,
    records: I,
    usage: ProviderUsage,
    invocation_arn: Option<&Field>,
) -> Result<EmbeddingReply, EmbeddingIoError>
where
    I: IntoIterator<Item = Result<(u32, EmbeddingVector), EmbeddingIoError>>,
{
    let arn = match invocation_arn {
        Some(field) => encode_json(field)?,
        None => Json::Null,
    };
    Ok(finish_reply(
        RESPONSE_OBJECT,
        model,
        embedding_records(records)?,
        EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: usage.prompt_tokens_details,
        },
        arn,
    ))
}

fn finish_reply(
    object: &'static str,
    model: String,
    data: Vec<EmbeddingRecord>,
    usage: EmbeddingUsage,
    invocation_arn: Json,
) -> EmbeddingReply {
    EmbeddingReply {
        object,
        data,
        model,
        usage,
        invocation_arn,
    }
}

// Nova declares embedding as list | str, and the shared record carries only those two shapes
fn embedding_vector(field: &Field) -> Result<EmbeddingVector, EmbeddingIoError> {
    match field {
        Field::Text(encoded) => Ok(EmbeddingVector::Base64(encoded.clone())),
        Field::List(items) => Ok(EmbeddingVector::Floats(number_vector(items)?)),
        Field::Null | Field::Bool(_) | Field::Int(_) | Field::Float(_) | Field::Mapping(_) => {
            Err(EmbeddingIoError::MalformedEmbedding)
        }
    }
}

fn number_vector(items: &[Field]) -> Result<Vec<Number>, EmbeddingIoError> {
    let mut numbers = Vec::with_capacity(items.len());
    for item in items {
        let number = match item {
            Field::Int(value) => Number::from(*value),
            Field::Float(value) => {
                Number::from_f64(*value).ok_or(EmbeddingIoError::Unrepresentable)?
            }
            Field::Null | Field::Bool(_) | Field::Text(_) | Field::List(_) | Field::Mapping(_) => {
                return Err(EmbeddingIoError::NotANumber);
            }
        };
        numbers.push(number);
    }
    Ok(numbers)
}

use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_prompts::arguments::json::Json;
use serde_json::Number;

use super::convert::encode_json;
use super::error::EmbeddingIoError;
use super::response::{EmbeddingReply, EmbeddingUsage, PromptTokenCounts};
use crate::image_models::params::Field;
use crate::marengo_embeddings::response::{self as marengo, MarengoEmbeddingResponse};
use crate::nova_embeddings::response::{self as nova, NovaEmbeddingResponse};
use crate::titan_embeddings::response::TitanEmbeddingResponse;

const RESPONSE_OBJECT: &str = "list";
const EMBEDDING_OBJECT: &str = "embedding";

pub(super) fn titan_embedding_reply(response: TitanEmbeddingResponse) -> EmbeddingReply {
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

pub(super) enum ProviderResponse {
    Nova(NovaEmbeddingResponse),
    Marengo(MarengoEmbeddingResponse),
}

#[derive(Clone, Copy)]
struct ProviderUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_tokens_details: Option<PromptTokenCounts>,
}

pub(super) fn provider_embedding_reply(
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
    ProviderUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: 0,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: details,
    }
}

fn marengo_usage(usage: marengo::MarengoUsage) -> ProviderUsage {
    let details = usage.prompt_tokens_details.map(|counts| PromptTokenCounts {
        image_count: counts.image_count,
        query_count: counts.query_count,
    });
    ProviderUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: details,
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
    Ok(EmbeddingReply {
        object: RESPONSE_OBJECT,
        model,
        data: embedding_records(records)?,
        usage: EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: usage.prompt_tokens_details,
        },
        invocation_arn: arn,
    })
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

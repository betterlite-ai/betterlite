use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

use super::convert::{parse_list, parse_one};
use super::error::EmbeddingIoError;
use super::response_projection::{
    ProviderResponse, provider_embedding_reply, titan_embedding_reply,
};
use crate::aws_endpoints::provider::BedrockEmbeddingProvider;
use crate::image_models::params::Params;
use crate::marengo_embeddings::response::{
    self as marengo, MarengoBilledRequest, MarengoInvokeResponse,
};
use crate::nova_embeddings::response as nova;
use crate::titan_embeddings::response as titan;
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
        if matches!(
            provider,
            BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon
        ) {
            return Ok(EmbeddingReply {
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
            });
        }
        return provider_response(provider, model, response_list, Some(reply), batch_data);
    }
    if let Some(reply) = titan_response(response_list, model, batch_data)? {
        return Ok(reply);
    }
    provider_response(provider, model, response_list, None, batch_data)
}

fn titan_response(
    response_list: &[Object],
    model: &str,
    batch_data: Option<&[Object]>,
) -> Result<Option<EmbeddingReply>, EmbeddingIoError> {
    match model {
        TITAN_MULTIMODAL_MODEL => {
            let response = multimodal::transform_response(response_list, model, batch_data)?;
            Ok(Some(titan_embedding_reply(response)))
        }
        TITAN_G1_MODEL | TITAN_G1_LEGACY_MODEL => {
            let response = titan::transform_response(response_list, model, titan::plain_embedding)?;
            Ok(Some(titan_embedding_reply(response)))
        }
        TITAN_V2_MODEL => {
            let response = titan::transform_response(response_list, model, v2::by_type_embedding)?;
            Ok(Some(titan_embedding_reply(response)))
        }
        _ => Ok(None),
    }
}

fn provider_response(
    provider: BedrockEmbeddingProvider,
    model: &str,
    response_list: &[Object],
    async_reply: Option<&Object>,
    batch_data: Option<&[Object]>,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    match provider {
        BedrockEmbeddingProvider::Twelvelabs => match async_reply {
            Some(reply) => {
                let input = parse_one::<Params>(reply)?;
                let response = marengo::transform_async_response(&input, model);
                provider_embedding_reply(ProviderResponse::Marengo(response))
            }
            None => {
                let replies = parse_list::<MarengoInvokeResponse>(response_list)?;
                let batch = batch_data
                    .map(parse_list::<MarengoBilledRequest>)
                    .transpose()?;
                let response = marengo::transform_response(&replies, model, batch.as_deref())?;
                provider_embedding_reply(ProviderResponse::Marengo(response))
            }
        },
        BedrockEmbeddingProvider::Nova => match async_reply {
            Some(reply) => {
                let input = parse_one::<Params>(reply)?;
                let response = nova::transform_invoke_response(&input, model);
                provider_embedding_reply(ProviderResponse::Nova(response))
            }
            None => {
                let replies = parse_list::<Params>(response_list)?;
                let batch = batch_data.map(parse_list::<Params>).transpose()?;
                let response = nova::transform_response(&replies, model, batch.as_deref())?;
                provider_embedding_reply(ProviderResponse::Nova(response))
            }
        },
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => {
            Err(EmbeddingIoError::Unmapped {
                model: model.to_owned(),
            })
        }
    }
}

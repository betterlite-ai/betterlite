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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_count: Option<u64>,
}

pub fn transform_response(
    response_list: &[Object],
    model: &str,
    provider: BedrockEmbeddingProvider,
    is_async_invoke: bool,
    batch_data: Option<&[Object]>,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    if let [reply] = response_list
        && is_async_invoke
        && reply.contains_key("invocationArn")
    {
        return async_reply(reply, model, provider);
    }
    if let Some(response) = titan_response(response_list, model, batch_data)? {
        return Ok(titan_embedding_reply(response));
    }
    let response = match provider {
        BedrockEmbeddingProvider::Twelvelabs => {
            let replies = parse_list::<MarengoInvokeResponse>(response_list)?;
            let batch = batch_data
                .map(parse_list::<MarengoBilledRequest>)
                .transpose()?;
            ProviderResponse::Marengo(marengo::transform_response(
                &replies,
                model,
                batch.as_deref(),
            )?)
        }
        BedrockEmbeddingProvider::Nova => {
            let replies = parse_list::<Params>(response_list)?;
            let batch = batch_data.map(parse_list::<Params>).transpose()?;
            ProviderResponse::Nova(nova::transform_response(&replies, model, batch.as_deref())?)
        }
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => {
            return Err(EmbeddingIoError::Unmapped {
                model: model.to_owned(),
            });
        }
    };
    provider_embedding_reply(response)
}

fn async_reply(
    reply: &Object,
    model: &str,
    provider: BedrockEmbeddingProvider,
) -> Result<EmbeddingReply, EmbeddingIoError> {
    match provider {
        BedrockEmbeddingProvider::Twelvelabs => {
            let response = marengo::transform_async_response(&parse_one::<Params>(reply)?, model);
            provider_embedding_reply(ProviderResponse::Marengo(response))
        }
        BedrockEmbeddingProvider::Nova => {
            let response = nova::transform_invoke_response(&parse_one::<Params>(reply)?, model);
            provider_embedding_reply(ProviderResponse::Nova(response))
        }
        BedrockEmbeddingProvider::Cohere | BedrockEmbeddingProvider::Amazon => Ok(EmbeddingReply {
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
    }
}

fn titan_response(
    response_list: &[Object],
    model: &str,
    batch_data: Option<&[Object]>,
) -> Result<Option<TitanEmbeddingResponse>, EmbeddingIoError> {
    let response = match model {
        TITAN_MULTIMODAL_MODEL => multimodal::transform_response(response_list, model, batch_data)?,
        TITAN_G1_MODEL | TITAN_G1_LEGACY_MODEL => {
            titan::transform_response(response_list, model, titan::plain_embedding)?
        }
        TITAN_V2_MODEL => titan::transform_response(response_list, model, v2::by_type_embedding)?,
        _ => return Ok(None),
    };
    Ok(Some(response))
}

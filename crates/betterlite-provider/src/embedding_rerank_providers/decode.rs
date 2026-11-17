use std::io::Error as IoError;

use betterlite_context::chat_config_selection::cohere::CohereRoute;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::cohere_chat_v1::reply::{ReplyInput as V1ReplyInput, decode_cohere_chat_v1};
use betterlite_openai::cohere_chat_v2::reply::{
    AssistantReply, ReplyInput as V2ReplyInput, decode_cohere_chat_v2,
};
use betterlite_openai::cohere_embedding::response::{DecodeInput, transform_embedding_response};
use betterlite_openai::cohere_embedding::{EmbeddingLogging, PostCall};
use betterlite_openai::infinity_embedding::decode_infinity_embedding;
use betterlite_openai::jina_embedding::transform_embedding_response as transform_jina_response;
use betterlite_openai::voyage_embedding::decode_voyage_embedding_reply;
use uuid::Uuid;

use super::error::FamilyError;
use super::request::plain;
use super::rerank;
use super::{
    CohereChatDecode, CohereEmbeddingDecode, Decode, EmbeddingRerankResult, JinaEmbeddingDecode,
};
use crate::operation::OperationError;

pub(super) fn answer(
    decode: &Decode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, OperationError> {
    decoded(decode, response).map_err(decode_error)
}

fn decoded(
    decode: &Decode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    match decode {
        Decode::CohereChat(chat) => cohere_chat(chat, response),
        Decode::CohereEmbedding(embedding) => cohere_embedding(embedding, response),
        Decode::VoyageEmbedding(variant) => {
            let reply = decode_voyage_embedding_reply(&response.body, response.status, *variant)?;
            Ok(EmbeddingRerankResult::VoyageEmbedding(Box::new(plain(
                &reply,
            )?)))
        }
        Decode::JinaEmbedding(jina) => jina_embedding(jina, response),
        Decode::InfinityEmbedding => Ok(EmbeddingRerankResult::InfinityEmbedding(Box::new(
            decode_infinity_embedding(&response.body, response.status)?,
        ))),
        Decode::Rerank(config) => rerank_answer(*config, response),
    }
}

// the framework carries a JSON decode failure, so the leaf's own message travels in the source text
fn decode_error(error: FamilyError) -> OperationError {
    OperationError::Decode {
        operation: "embedding_rerank",
        source: serde_json::Error::io(IoError::other(error.to_string())),
    }
}

fn rerank_answer(
    config: RerankConfig,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    let answer = rerank::decode(config, response.status, &response.body)?;
    Ok(EmbeddingRerankResult::Rerank(Box::new(answer)))
}

fn cohere_chat(
    chat: &CohereChatDecode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    let answer = match chat.version {
        CohereRoute::V1 => {
            EmbeddingRerankResult::CohereV1Chat(Box::new(decode_cohere_chat_v1(V1ReplyInput {
                body: &response.body,
                status: response.status,
                model: &chat.model,
                created: chat.created,
                id: chat.id.clone(),
                finish_reason: None,
            })?))
        }
        CohereRoute::V2 => {
            EmbeddingRerankResult::CohereV2Chat(Box::new(decode_cohere_chat_v2(V2ReplyInput {
                body: &response.body,
                status: response.status,
                model: &chat.model,
                created: chat.created,
                id: chat.id.clone(),
                finish_reason: None,
                message: AssistantReply::default(),
                // LiteLLM mints an id for a tool call the reply left without one
                next_id: || Uuid::new_v4().to_string(),
            })?))
        }
    };
    Ok(answer)
}

fn cohere_embedding(
    embedding: &CohereEmbeddingDecode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    let mut logging = DecodeLogging {
        input: embedding.input.clone(),
    };
    let answer = transform_embedding_response(
        DecodeInput {
            body: &response.body,
            model: &embedding.model,
            api_key: embedding.api_key.as_ref(),
            request_data: &embedding.request_data,
            encoding: &embedding.encoding,
        },
        &mut logging,
    )?;
    Ok(EmbeddingRerankResult::CohereEmbedding(Box::new(answer)))
}

fn jina_embedding(
    jina: &JinaEmbeddingDecode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    let answer = transform_jina_response(
        &response.body,
        jina.api_key.as_ref(),
        &jina.request_data,
        |_call| {},
    )?;
    Ok(EmbeddingRerankResult::JinaEmbedding(Box::new(answer)))
}

// LiteLLM reads the input from the logging details, which is the only detail this transform uses
struct DecodeLogging {
    input: Vec<String>,
}

impl EmbeddingLogging for DecodeLogging {
    fn input(&self) -> &[String] {
        &self.input
    }

    fn post_call(&mut self, _call: PostCall<'_>) {}
}

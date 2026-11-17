use std::io::Error as IoError;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::cohere_embedding::response::{DecodeInput, transform_embedding_response};
use betterlite_openai::cohere_embedding::{EmbeddingLogging, PostCall};
use betterlite_openai::infinity_embedding::decode_infinity_embedding;
use betterlite_openai::jina_embedding::transform_embedding_response as transform_jina_response;
use betterlite_openai::voyage_embedding::decode_voyage_embedding_reply;

use super::error::FamilyError;
use super::rerank;
use super::{Decode, EmbeddingRerankResult, chat};
use crate::operation::OperationError;

// the framework carries a JSON decode failure, so the leaf's typed error travels as its cause
pub(super) fn answer(
    decode: &Decode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, OperationError> {
    decoded(decode, response).map_err(|error| OperationError::Decode {
        operation: "embedding_rerank",
        source: serde_json::Error::io(IoError::other(error)),
    })
}

fn decoded(
    decode: &Decode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    match decode {
        Decode::CohereChat(chat) => chat::decode(chat, response),
        Decode::CohereEmbedding(embedding) => Ok(EmbeddingRerankResult::CohereEmbedding(Box::new(
            transform_embedding_response(
                DecodeInput {
                    body: &response.body,
                    model: &embedding.model,
                    api_key: embedding.api_key.as_ref(),
                    request_data: &embedding.request_data,
                    encoding: &embedding.encoding,
                },
                &mut DecodeLogging {
                    input: embedding.input.clone(),
                },
            )?,
        ))),
        Decode::VoyageEmbedding(variant) => {
            let reply = decode_voyage_embedding_reply(&response.body, response.status, *variant)?;
            let answer = serde_json::from_value(serde_json::to_value(&reply)?)?;
            Ok(EmbeddingRerankResult::VoyageEmbedding(Box::new(answer)))
        }
        Decode::JinaEmbedding(jina) => Ok(EmbeddingRerankResult::JinaEmbedding(Box::new(
            transform_jina_response(
                &response.body,
                jina.api_key.as_ref(),
                &jina.request_data,
                |_call| {},
            )?,
        ))),
        Decode::InfinityEmbedding => Ok(EmbeddingRerankResult::InfinityEmbedding(Box::new(
            decode_infinity_embedding(&response.body, response.status)?,
        ))),
        Decode::Rerank(config) => rerank::decode(*config, response.status, &response.body)
            .map(|reply| EmbeddingRerankResult::Rerank(Box::new(reply))),
    }
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

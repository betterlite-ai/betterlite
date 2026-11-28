use betterlite_bedrock::count_call::counter::TokenCountResponse;
use betterlite_bedrock::embedding_call::EmbeddingCallResponse;
use betterlite_bedrock::transcription::binding::TranscriptionResponse;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;

use crate::bedrock_image_dispatch::BedrockImageResult;

pub mod call;
pub mod error;
pub mod execute;
// The fixture transport every family's tests share; the oracle adapter builds it through the testing feature
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

#[cfg(test)]
mod tests;

// B010's arm carries no data because OperationRequest is built before a non-chat call reaches its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockOtherRequest;

// One result per non-chat Bedrock leaf; each leaf's own record travels through unchanged, boxed to keep the enum small
pub enum BedrockOtherResult {
    Embedding(Box<EmbeddingCallResponse>),
    EmbeddingStatus(Box<Json>),
    Rerank(Box<RerankResponse>),
    Count(Option<Box<TokenCountResponse>>),
    Transcription(Box<TranscriptionResponse>),
    Image(Box<BedrockImageResult>),
}

// The prepared arm stays a marker: the dispatcher answers with the leaf's own record
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockOtherPrepared;

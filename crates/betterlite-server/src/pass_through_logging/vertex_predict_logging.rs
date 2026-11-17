use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_embedding_records::error::GeminiEmbeddingError;
use betterlite_vertex::multimodal_embedding_records::error::MultimodalEmbeddingError;
use betterlite_vertex::vertex_image_policy::error::PredictionError;
use betterlite_vertex::vertex_relay_metadata::types::CompletionCost;
use betterlite_vertex::vertex_text_embeddings::error::VertexTextEmbeddingError;
use http::{HeaderMap, StatusCode};
use thiserror::Error;

use counter::Cl100kCounter;

mod cost;
mod counter;
mod detect;
mod embed;
mod input;
mod predict;
mod spend;
mod wire;

mod tests;

pub struct VertexPredictInput<'input> {
    pub logging_obj: &'input StreamLoggingObject,
    pub url_route: &'input str,
    pub status: StatusCode,
    pub response_headers: &'input HeaderMap,
    pub response_body: &'input [u8],
    pub kwargs: PassThroughLoggingKwargs,
}

pub struct VertexEmbedContentInput<'input> {
    pub logging_obj: &'input StreamLoggingObject,
    pub url_route: &'input str,
    pub status: StatusCode,
    pub response_headers: &'input HeaderMap,
    pub response_body: &'input [u8],
    pub request_body: Option<&'input Json>,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum VertexPredictError {
    #[error("the Vertex response body is not valid JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex response body must be a JSON object, found {found}")]
    BodyShape { found: &'static str },
    #[error("the Vertex reply could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response cost of model {model} could not be resolved")]
    Cost { model: String },
    #[error("the bundled cl100k tokenizer could not be built")]
    Tokenizer,
    #[error(transparent)]
    Prediction(#[from] PredictionError),
    #[error(transparent)]
    Multimodal(#[from] MultimodalEmbeddingError),
    #[error(transparent)]
    TextEmbedding(#[from] VertexTextEmbeddingError),
    #[error(transparent)]
    GeminiEmbedding(#[from] GeminiEmbeddingError),
}

pub struct VertexPredictLogger {
    cost: Arc<dyn CompletionCost>,
    catalog: Arc<ModelCatalog>,
    counter: Option<Cl100kCounter>,
}

impl VertexPredictLogger {
    #[must_use]
    pub fn new(cost: Arc<dyn CompletionCost>) -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            catalog: bundled.get(),
            cost,
            counter: Cl100kCounter::bundled(),
        }
    }

    // ast-grep-ignore: no-one-line-functions - B3642 calls this fixed name with the fixed input type
    pub fn handle_predict_response(
        &self,
        input: VertexPredictInput<'_>,
    ) -> Result<HandlerOutcome, VertexPredictError> {
        predict::predict(&input, self.cost.as_ref(), &self.catalog)
    }

    // ast-grep-ignore: no-one-line-functions - B3642 calls this fixed name with the fixed input type
    pub fn handle_embed_content_response(
        &self,
        input: VertexEmbedContentInput<'_>,
    ) -> Result<HandlerOutcome, VertexPredictError> {
        embed::embed_content(&input, self.cost.as_ref(), self.counter.as_ref())
    }
}

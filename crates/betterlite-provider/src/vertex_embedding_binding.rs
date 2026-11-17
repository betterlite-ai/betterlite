use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_core::vertex_embedding_dispatch::{
    self, Defaults, Pinned, SelectionRequest, VertexEmbeddingDispatchError,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Extra;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_records::response::EmbeddingReply;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use betterlite_vertex::vertex_embedding_call::error::VertexEmbeddingCallError;
use secrecy::SecretString;
use thiserror::Error;

use self::arguments::{EmbedContentArguments, MultimodalArguments, TextArguments, arguments};

pub mod adapter;
pub mod arguments;
pub mod calls;
mod tests;

// the framework's OperationRequest tests build this variant from its bare name, so it stays a unit; the call's data is VertexEmbeddingInput
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VertexEmbeddingRequest;

#[derive(Debug, Clone)]
pub struct VertexEmbeddingInput {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Extra,
    pub litellm_params: Option<Extra>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub pinned: Option<Pinned>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub aembedding: bool,
}

#[derive(Debug, Clone)]
pub enum VertexEmbeddingPrepared {
    EmbedContent(Box<EmbedContentArguments>),
    Multimodal(Box<MultimodalArguments>),
    Text(Box<TextArguments>),
}

pub enum VertexEmbeddingResult {
    EmbedContent(EmbeddingReply),
    Multimodal(MultimodalEmbeddingAnswer),
    Text(EmbeddingResponse),
}

#[derive(Debug, Error)]
pub enum VertexEmbeddingPrepareError {
    #[error(transparent)]
    Dispatch(#[from] VertexEmbeddingDispatchError),
    #[error("the embedContent API takes text input, and the request sent tokens")]
    TokenInput,
}

#[derive(Debug, Error)]
pub enum VertexEmbeddingError {
    #[error(transparent)]
    EmbedContent(#[from] GeminiEmbeddingCallError),
    #[error(transparent)]
    Prediction(#[from] VertexEmbeddingCallError),
}

pub type EmbedContentOutcome = Result<EmbeddingReply, GeminiEmbeddingCallError>;
pub type MultimodalOutcome = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>;
pub type TextOutcome = Result<EmbeddingResponse, VertexEmbeddingCallError>;

pub fn prepare(
    request: VertexEmbeddingInput,
    defaults: Defaults<'_>,
    catalog: &ModelCatalog,
) -> Result<VertexEmbeddingPrepared, VertexEmbeddingPrepareError> {
    let resolved = vertex_embedding_dispatch::select(
        SelectionRequest {
            model: &request.model,
            api_base: request.api_base.as_deref(),
            optional_params: &request.optional_params,
            pinned: request.pinned.as_ref(),
        },
        defaults,
        catalog,
    )?;
    arguments(request, resolved)
}

// the three embedding handlers main.py calls, as the arguments each signature receives
pub trait VertexEmbeddingHandlers {
    fn embed_content(
        &self,
        call: &EmbedContentArguments,
    ) -> impl Future<Output = EmbedContentOutcome> + Send;

    fn multimodal(
        &self,
        call: &MultimodalArguments,
    ) -> impl Future<Output = MultimodalOutcome> + Send;

    fn text(&self, call: &TextArguments) -> impl Future<Output = TextOutcome> + Send;
}

pub async fn execute<Handlers>(
    prepared: &VertexEmbeddingPrepared,
    handlers: &Handlers,
) -> Result<VertexEmbeddingResult, VertexEmbeddingError>
where
    Handlers: VertexEmbeddingHandlers + Sync,
{
    let result = match prepared {
        VertexEmbeddingPrepared::EmbedContent(call) => {
            VertexEmbeddingResult::EmbedContent(handlers.embed_content(call).await?)
        }
        VertexEmbeddingPrepared::Multimodal(call) => {
            VertexEmbeddingResult::Multimodal(handlers.multimodal(call).await?)
        }
        VertexEmbeddingPrepared::Text(call) => {
            VertexEmbeddingResult::Text(handlers.text(call).await?)
        }
    };
    Ok(result)
}

pub mod config;
pub mod error;
pub mod live;
pub mod params;
pub mod ports;
pub mod query;
pub mod reply;
mod tests;
mod wire;

use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_provider::api::vector_store_contracts::config::{Params, SearchQuery};
use betterlite_redis::valkey::pack_vector;

use error::ValkeyError;
use params::ValkeyParams;
use ports::{
    ValkeyConnector, ValkeyEmbedder, ValkeySearchParams, ValkeySearchPort, ValkeyTimeout,
    socket_timeouts,
};
use query::{knn_query, query_text};

pub struct ValkeySearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub timeout: ValkeyTimeout,
}

pub async fn search(
    request: &ValkeySearchRequest<'_>,
    embedder: &dyn ValkeyEmbedder,
    connector: &dyn ValkeyConnector,
    ready_port: Option<&dyn ValkeySearchPort>,
) -> Result<VectorStoreSearchResponse, ValkeyError> {
    let params = ValkeyParams::read(request.litellm_params)?;
    let text = query_text(request.query)?;
    let knn = knn_query(
        request.optional_params,
        &params.embedding_field,
        &params.text_field,
    )?;
    let model = params
        .embedding_model
        .as_deref()
        .ok_or(ValkeyError::EmbeddingModelRequired)?;
    let embeddings = embedder
        .embed(model, &text, &params.embedding_config)
        .await
        .map_err(|source| ValkeyError::Embedding { source })?;
    let vector = embeddings.first().ok_or(ValkeyError::NoEmbedding)?;
    let mut search_params = ValkeySearchParams::new();
    search_params.insert("vec".to_owned(), pack_vector(vector));
    let documents = if let Some(port) = ready_port {
        port.search(request.vector_store_id, &knn, &search_params)
            .await?
    } else {
        let url = params.connection_url()?;
        let port = connector
            .connect(&url, socket_timeouts(request.timeout))
            .await?;
        port.search(request.vector_store_id, &knn, &search_params)
            .await?
    };
    reply::search_response(&documents, &text, &params.text_field)
}

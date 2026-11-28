use std::time::Duration;

use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::embeddings::call::{
    EmbeddingClientFactory, FoundryEmbeddingRequest, TextEmbedding, embedding,
};
use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_foundry::embeddings::reply::FoundryEmbeddingResponse;
use betterlite_foundry::embeddings::request::FoundryEmbeddingOptions;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use thiserror::Error;

use super::credentials::{Globals, resolve_embedding_key};

pub struct Call<'data> {
    pub model: &'data str,
    pub input: &'data [String],
    pub options: FoundryEmbeddingOptions,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub params: &'data Object,
    pub timeout: Duration,
    pub mode: ClientKind,
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error(transparent)]
    Embedding(#[from] FoundryEmbeddingError),
}

// B552 owns both embedding routes; this branch only hands it the base and key main.py resolves first
pub async fn dispatch<Factory, Text>(
    call: &Call<'_>,
    globals: &Globals<'_>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    factory: &Factory,
    text: &Text,
) -> Result<FoundryEmbeddingResponse, DispatchError>
where
    Factory: EmbeddingClientFactory,
    Text: TextEmbedding,
{
    let api_base = globals
        .accessors()
        .api_base(call.api_base.filter(|given| !given.is_empty()));
    let api_key = resolve_embedding_key(call.api_key, call.params, globals)?;
    let request = FoundryEmbeddingRequest {
        model: call.model,
        input: call.input,
        options: call.options.clone(),
        api_base: api_base.as_deref(),
        api_key: api_key.as_ref(),
        timeout: call.timeout,
        mode: call.mode,
    };
    Ok(embedding(&request, supplied, factory, text).await?)
}

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_core::operation::embedding::{
    EmbeddingCall, EmbeddingCallInput, EmbeddingFailure, EmbeddingResult,
};
use betterlite_core::vertex_embedding_dispatch::{Defaults, Pinned};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::{
    EmbeddingError, EmbeddingResponse, EmbeddingUsage, EncodingFormat,
};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_call::transport::EmbeddingTransport;
use betterlite_vertex::gemini_embedding_records::response::EmbeddingReply;
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_refresh::CredentialRefresh;
use betterlite_vertex::vertex_embedding_call::error::VertexEmbeddingCallError;
use betterlite_vertex::vertex_embedding_call::post::EmbeddingPost;
use betterlite_vertex::vertex_text_embeddings::request::map_openai_params;
use http::HeaderMap;
use serde_json::Number;

use super::adapter::VertexEmbeddingAdapter;
use super::{
    VertexEmbeddingError, VertexEmbeddingInput, VertexEmbeddingPrepareError, VertexEmbeddingResult,
    execute, prepare,
};

mod tests;

// a request body never names the credentials the gateway signs with, as LiteLLM bans vertex_credentials there
const CLIENT_BARRED: [&str; 2] = ["vertex_credentials", "vertex_ai_credentials"];

pub struct VertexEmbeddingCalls<Transport> {
    pub catalog: Arc<ModelCatalog>,
    pub cache: Arc<CredentialCache>,
    pub refresh: Arc<CredentialRefresh>,
    pub counter: Arc<dyn TextTokenCounter + Send + Sync>,
    pub transport: Arc<Transport>,
    pub vertex: VertexSettings,
    pub embedding: EmbeddingSettings,
    pub pinned: BTreeMap<String, Pinned>,
}

impl<Transport> VertexEmbeddingCalls<Transport> {
    // LiteLLM's vertex_ai branch supports dimensions alone, takes float as a no-op and refuses the rest without drop_params
    fn optional_params(call: &EmbeddingCallInput<'_>) -> Result<Extra, EmbeddingFailure> {
        if call.request.encoding_format == Some(EncodingFormat::Base64) {
            return Err(unsupported("encoding_format"));
        }
        if call.request.user.is_some() {
            return Err(unsupported("user"));
        }
        let internal = |source: serde_json::Error| EmbeddingFailure::Internal {
            message: source.to_string(),
        };
        let encoded = serde_json::to_vec(&call.request.passthrough).map_err(internal)?;
        let kwargs: Extra = serde_json::from_slice(&encoded).map_err(internal)?;
        if let Some(name) = kwargs
            .keys()
            .find(|name| CLIENT_BARRED.contains(&name.as_str()))
        {
            return Err(unsupported(name));
        }
        let mut non_default = Extra::new();
        if let Some(dimensions) = call.request.dimensions {
            non_default.insert(
                "dimensions".to_owned(),
                Json::Number(Number::from(dimensions)),
            );
        }
        let (mut optional_params, remaining) =
            map_openai_params(&non_default, Extra::new(), kwargs);
        optional_params.extend(remaining);
        Ok(optional_params)
    }

    fn input(
        &self,
        call: &EmbeddingCallInput<'_>,
    ) -> Result<VertexEmbeddingInput, EmbeddingFailure> {
        let deployment = &call.deployment;
        let upstream = deployment
            .litellm_model
            .split_once('/')
            .map_or(deployment.litellm_model.as_str(), |(_provider, model)| {
                model
            });
        Ok(VertexEmbeddingInput {
            model: upstream.to_owned(),
            input: call.request.input.clone(),
            optional_params: Self::optional_params(call)?,
            litellm_params: None,
            api_key: deployment.api_key.clone(),
            api_base: None,
            pinned: self.pinned.get(&deployment.model_id).cloned(),
            extra_headers: BTreeMap::new(),
            timeout: Some(deployment.timeout),
            aembedding: true,
        })
    }
}

#[async_trait]
impl<Transport> EmbeddingCall for VertexEmbeddingCalls<Transport>
where
    Transport: EmbeddingTransport + EmbeddingPost + 'static,
{
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure> {
        let defaults = Defaults {
            vertex: &self.vertex,
            embedding: &self.embedding,
        };
        let prepared = prepare(self.input(&call)?, defaults, &self.catalog)?;
        let adapter = VertexEmbeddingAdapter {
            catalog: &self.catalog,
            cache: &self.cache,
            refresh: &self.refresh,
            counter: &*self.counter,
            pooled: &*self.transport,
            supplied: None,
        };
        let started = Instant::now();
        let result = execute(&prepared, &adapter).await?;
        Ok(EmbeddingResult {
            response: EmbeddingResponse::from(result),
            upstream_headers: HeaderMap::new(),
            upstream_time: started.elapsed(),
        })
    }
}

fn unsupported(name: &str) -> EmbeddingFailure {
    EmbeddingFailure::Route(EmbeddingError::Unsupported {
        name: name.to_owned(),
    })
}

fn provider_status(status: u16, body: Vec<u8>) -> EmbeddingFailure {
    EmbeddingFailure::Route(EmbeddingError::Provider {
        response: Box::new(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }),
    })
}

// an upstream refusal keeps its status and body for the route's error rendering
impl From<VertexEmbeddingError> for EmbeddingFailure {
    fn from(error: VertexEmbeddingError) -> Self {
        match error {
            VertexEmbeddingError::Prediction(VertexEmbeddingCallError::Status { status, body }) => {
                provider_status(status, body)
            }
            VertexEmbeddingError::EmbedContent(GeminiEmbeddingCallError::PostStatus {
                status,
                body,
            }) => provider_status(status, body.into_bytes()),
            other @ (VertexEmbeddingError::EmbedContent(_)
            | VertexEmbeddingError::Prediction(_)) => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

impl From<VertexEmbeddingPrepareError> for EmbeddingFailure {
    fn from(error: VertexEmbeddingPrepareError) -> Self {
        if matches!(error, VertexEmbeddingPrepareError::TokenInput) {
            return unsupported("input");
        }
        Self::Internal {
            message: error.to_string(),
        }
    }
}

fn reply_response(reply: EmbeddingReply) -> EmbeddingResponse {
    let usage = reply.usage;
    EmbeddingResponse {
        object: "list".to_owned(),
        data: reply.data,
        model: reply.model,
        usage: EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: usage.total_tokens,
        },
    }
}

impl From<VertexEmbeddingResult> for EmbeddingResponse {
    fn from(result: VertexEmbeddingResult) -> Self {
        match result {
            VertexEmbeddingResult::EmbedContent(reply) => reply_response(reply),
            VertexEmbeddingResult::Multimodal(answer) => answer.response,
            VertexEmbeddingResult::Text(response) => response,
        }
    }
}

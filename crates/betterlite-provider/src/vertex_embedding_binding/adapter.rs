use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::catalog::ModelCatalog;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_vertex::endpoint_target::target::TargetProvider;
use betterlite_vertex::gemini_embedding_call::call::{EmbeddingCall, Execution};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_call::execute::{
    Clients as EmbedContentClients, Dependencies as EmbedContentDependencies, gemini_embedding_call,
};
use betterlite_vertex::gemini_embedding_call::transport::EmbeddingTransport;
use betterlite_vertex::gemini_embedding_records::response::EmbeddingReply;
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use betterlite_vertex::model_policy::capabilities::VertexCapabilityProvider;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_refresh::CredentialRefresh;
use betterlite_vertex::vertex_embedding_call::error::VertexEmbeddingCallError;
use betterlite_vertex::vertex_embedding_call::multimodal::{
    MultimodalCall, MultimodalClients, multimodal_embedding,
};
use betterlite_vertex::vertex_embedding_call::post::EmbeddingPost;
use betterlite_vertex::vertex_embedding_call::text::{
    TextAuth, TextCall, TextClients, text_embedding,
};

use super::VertexEmbeddingHandlers;
use super::arguments::{EmbedContentArguments, MultimodalArguments, TextArguments};

// main.py sends no headers to the prediction handlers, whose own defaults are empty
const NO_HEADERS: BTreeMap<String, String> = BTreeMap::new();

// the production handlers: B1341's embedContent call and B1344's two prediction calls
pub struct VertexEmbeddingAdapter<'deps, Transport> {
    pub catalog: &'deps ModelCatalog,
    pub cache: &'deps CredentialCache,
    pub refresh: &'deps CredentialRefresh,
    pub counter: &'deps (dyn TextTokenCounter + Sync),
    pub pooled: &'deps Transport,
    pub supplied: Option<&'deps Transport>,
}

impl<Transport> VertexEmbeddingHandlers for VertexEmbeddingAdapter<'_, Transport>
where
    Transport: EmbeddingTransport + EmbeddingPost,
{
    fn embed_content<'call>(
        &'call self,
        call: &'call EmbedContentArguments,
    ) -> impl Future<Output = Result<EmbeddingReply, GeminiEmbeddingCallError>> + Send + 'call {
        async move {
            let embedding_call = EmbeddingCall {
                provider: TargetProvider::Vertex,
                execution: execution(call.aembedding),
                model: &call.model,
                input: &call.input,
                optional_params: &call.optional_params,
                api_key: None,
                api_base: call.api_base.as_deref(),
                credentials: call.credentials.as_ref(),
                vertex_project: call.project.as_deref(),
                vertex_location: call.location.as_deref(),
                timeout: None,
                extra_headers: &call.extra_headers,
            };
            let dependencies = EmbedContentDependencies {
                catalog: self.catalog,
                mint: self.cache,
                counter: self.counter,
                clients: EmbedContentClients {
                    supplied: self.supplied,
                    pooled: self.pooled,
                },
            };
            gemini_embedding_call(&embedding_call, &dependencies).await
        }
    }

    fn multimodal<'call>(
        &'call self,
        call: &'call MultimodalArguments,
    ) -> impl Future<Output = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            let multimodal_call = MultimodalCall {
                provider: VertexCapabilityProvider::VertexAi,
                model: &call.model,
                input: &call.input,
                optional_params: &call.optional_params,
                headers: &NO_HEADERS,
                api_base: call.api_base.as_deref(),
                gemini_api_key: None,
                project: call.project.as_deref(),
                location: call.location.as_deref(),
                credentials: call.credentials.as_ref(),
                timeout: None,
                aembedding: call.aembedding,
            };
            let clients = MultimodalClients {
                pooled: self.pooled,
                caller: self.supplied.map(|supplied| supplied as &dyn EmbeddingPost),
            };
            multimodal_embedding(&multimodal_call, self.catalog, self.cache, &clients).await
        }
    }

    fn text<'call>(
        &'call self,
        call: &'call TextArguments,
    ) -> impl Future<Output = Result<EmbeddingResponse, VertexEmbeddingCallError>> + Send + 'call
    {
        async move {
            let text_call = TextCall {
                provider: VertexCapabilityProvider::VertexAi,
                model: &call.model,
                input: &call.input,
                optional_params: &call.optional_params,
                litellm_params: call.litellm_params.as_ref(),
                api_base: call.api_base.as_deref(),
                gemini_api_key: call.api_key.as_ref(),
                project: call.project.as_deref(),
                location: call.location.as_deref(),
                credentials: call.credentials.as_ref(),
                extra_headers: &NO_HEADERS,
                timeout: call.timeout,
                aembedding: call.aembedding,
            };
            let auth = TextAuth {
                cache: self.cache,
                refresh: self.refresh,
            };
            let clients = TextClients {
                pooled: self.pooled,
                caller: self.supplied.map(|supplied| supplied as &dyn EmbeddingPost),
            };
            text_embedding(&text_call, self.catalog, &auth, &clients).await
        }
    }
}

const fn execution(aembedding: bool) -> Execution {
    if aembedding {
        Execution::Async
    } else {
        Execution::Sync
    }
}

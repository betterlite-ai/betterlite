use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::vertex_embedding_dispatch::{
    self, Defaults, Native, Resolution, SelectionRequest, VertexEmbeddingDispatchError,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_vertex::endpoint_target::target::TargetProvider;
use betterlite_vertex::gemini_embedding_call::call::{EmbeddingCall, Execution};
use betterlite_vertex::gemini_embedding_call::error::GeminiEmbeddingCallError;
use betterlite_vertex::gemini_embedding_call::execute::{
    Clients as EmbedContentClients, Dependencies as EmbedContentDependencies, gemini_embedding_call,
};
use betterlite_vertex::gemini_embedding_call::transport::EmbeddingTransport;
use betterlite_vertex::gemini_embedding_records::input::{
    EmbeddingInput as EmbedContentInput, InputEntry,
};
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
use secrecy::SecretString;
use serde_json::Number;
use thiserror::Error;

mod tests;

// main.py sends no headers to the prediction handlers, whose own defaults are empty
const NO_HEADERS: BTreeMap<String, String> = BTreeMap::new();

#[derive(Debug, Clone)]
pub struct VertexEmbeddingRequest {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Extra,
    pub litellm_params: Option<Extra>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub aembedding: bool,
}

#[derive(Debug, Clone)]
pub enum VertexEmbeddingPrepared {
    EmbedContent(EmbedContentArguments),
    Multimodal(MultimodalArguments),
    Text(TextArguments),
}

// main.py's batch_embeddings call: api_key=None and no timeout, with the caller's headers
#[derive(Debug, Clone)]
pub struct EmbedContentArguments {
    pub model: String,
    pub input: EmbedContentInput,
    pub optional_params: Extra,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub aembedding: bool,
}

// main.py's multimodal_embedding call: litellm_params and no timeout, no headers and no api_key.
// The handler reads litellm_params only where LiteLLM ignores it.
#[derive(Debug, Clone)]
pub struct MultimodalArguments {
    pub model: String,
    pub input: Json,
    pub optional_params: Extra,
    pub litellm_params: Option<Extra>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub aembedding: bool,
}

// main.py's vertex_embedding.embedding call: the timeout and api_key, no headers
#[derive(Debug, Clone)]
pub struct TextArguments {
    pub model: String,
    pub input: EmbeddingInput,
    pub optional_params: Json,
    pub litellm_params: Option<Json>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<Duration>,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    pub aembedding: bool,
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

pub fn prepare(
    request: VertexEmbeddingRequest,
    defaults: Defaults<'_>,
    catalog: &ModelCatalog,
) -> Result<VertexEmbeddingPrepared, VertexEmbeddingPrepareError> {
    let resolution = vertex_embedding_dispatch::select(
        SelectionRequest {
            model: &request.model,
            api_base: request.api_base.as_deref(),
            optional_params: &request.optional_params,
        },
        defaults,
        catalog,
    )?;
    let prepared = match resolution.native {
        Native::EmbedContent => {
            VertexEmbeddingPrepared::EmbedContent(embed_content_arguments(request, resolution)?)
        }
        Native::Multimodal => {
            VertexEmbeddingPrepared::Multimodal(multimodal_arguments(request, resolution))
        }
        Native::Text => VertexEmbeddingPrepared::Text(text_arguments(request, resolution)),
    };
    Ok(prepared)
}

fn embed_content_arguments(
    request: VertexEmbeddingRequest,
    resolution: Resolution,
) -> Result<EmbedContentArguments, VertexEmbeddingPrepareError> {
    Ok(EmbedContentArguments {
        input: embed_content_input(&request.input)?,
        model: request.model,
        optional_params: resolution.optional_params,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        extra_headers: request.extra_headers,
        aembedding: request.aembedding,
    })
}

fn multimodal_arguments(
    request: VertexEmbeddingRequest,
    resolution: Resolution,
) -> MultimodalArguments {
    MultimodalArguments {
        input: input_json(&request.input),
        model: request.model,
        optional_params: resolution.optional_params,
        litellm_params: request.litellm_params,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        aembedding: request.aembedding,
    }
}

fn text_arguments(request: VertexEmbeddingRequest, resolution: Resolution) -> TextArguments {
    TextArguments {
        model: request.model,
        input: request.input,
        optional_params: Json::Object(resolution.optional_params),
        litellm_params: request.litellm_params.map(Json::Object),
        api_key: request.api_key,
        timeout: request.timeout,
        project: resolution.project,
        location: resolution.location,
        credentials: resolution.credentials,
        api_base: resolution.api_base,
        aembedding: request.aembedding,
    }
}

fn embed_content_input(
    input: &EmbeddingInput,
) -> Result<EmbedContentInput, VertexEmbeddingPrepareError> {
    match input {
        EmbeddingInput::Text(text) => Ok(EmbedContentInput::Text(text.clone())),
        EmbeddingInput::Texts(texts) => Ok(EmbedContentInput::Entries(
            texts.iter().cloned().map(InputEntry::Text).collect(),
        )),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(VertexEmbeddingPrepareError::TokenInput)
        }
    }
}

fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Texts(texts) => Json::List(texts.iter().cloned().map(Json::Text).collect()),
        EmbeddingInput::Tokens(tokens) => Json::List(
            tokens
                .iter()
                .copied()
                .map(Number::from)
                .map(Json::Number)
                .collect(),
        ),
        EmbeddingInput::TokenLists(lists) => Json::List(
            lists
                .iter()
                .map(|list| {
                    Json::List(
                        list.iter()
                            .copied()
                            .map(Number::from)
                            .map(Json::Number)
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

// the three embedding handlers main.py calls, as the arguments each signature receives
pub trait VertexEmbeddingHandlers {
    fn embed_content<'call>(
        &'call self,
        call: &'call EmbedContentArguments,
    ) -> impl Future<Output = Result<EmbeddingReply, GeminiEmbeddingCallError>> + Send + 'call;

    fn multimodal<'call>(
        &'call self,
        call: &'call MultimodalArguments,
    ) -> impl Future<Output = Result<MultimodalEmbeddingAnswer, VertexEmbeddingCallError>> + Send + 'call;

    fn text<'call>(
        &'call self,
        call: &'call TextArguments,
    ) -> impl Future<Output = Result<EmbeddingResponse, VertexEmbeddingCallError>> + Send + 'call;
}

pub async fn execute<Handlers: VertexEmbeddingHandlers + Sync>(
    prepared: &VertexEmbeddingPrepared,
    handlers: &Handlers,
) -> Result<VertexEmbeddingResult, VertexEmbeddingError> {
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

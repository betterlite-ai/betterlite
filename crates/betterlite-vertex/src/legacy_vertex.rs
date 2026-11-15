use std::future::Future;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::message_utilities::response_streams::iterators::ModelResponseIterator;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::Object;
use uuid::Uuid;

use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::gemini_options::defaults::GenerationDefaults;
use crate::legacy_prediction::client::PredictionClient;
use crate::legacy_prediction::client::PredictionStream;
use crate::legacy_prediction::execution::{LegacyChunkStream, LegacyContext};
use crate::legacy_prediction::projection::{LegacyResult, ProjectionServices};
use crate::legacy_prediction::streamer::TextStreamer;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

pub mod cache;
mod delegation;
pub mod error;
mod fake_response;
mod plan;
mod sync;
mod tests;

use self::cache::ClientCache;
use self::error::LegacyVertexError;

pub struct LegacyRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub options: Object,
    pub acompletion: bool,
}

pub struct LegacyDeps<'config, 'encoding> {
    pub sets: &'config ProviderModelSets,
    pub defaults: &'config GenerationDefaults,
    pub services: &'config ProjectionServices<'encoding>,
    pub identifier: fn() -> Uuid,
    pub credentials: Option<&'config ConfiguredCredentials>,
    pub project: Option<&'config str>,
    pub location: Option<&'config str>,
}

// LiteLLM's non-Gemini handler returns an SDK stream, a word stream, the bedrock iterator, a ModelResponse or a coroutine's answer
pub enum LegacyCompletion {
    // Boxed to stay within 3x of FakeStream, the limit variant_size_differences enforces
    Response(Box<LegacyResult>),
    Prediction(PredictionStream),
    Chunks(LegacyChunkStream),
    Words(TextStreamer),
    FakeStream(ModelResponseIterator),
}

pub async fn complete<Cache, Client, Make, Lookup>(
    cache: &Cache,
    client: &Client,
    request: LegacyRequest<'_>,
    deps: LegacyDeps<'_, '_>,
    make_lookup: Make,
) -> Result<LegacyCompletion, LegacyVertexError>
where
    Cache: ClientCache<Client = Client> + Sync,
    Client: PredictionClient,
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup + Send,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let key = cache::cache_key(request.model, deps.project, deps.location);
    let cached = cache.cached(&key)?;
    let access = ensure_access_token(
        VertexCapabilityProvider::VertexAi,
        make_lookup(deps.credentials, deps.project),
    )
    .await?;
    let plan = plan::plan(request, &deps)?;
    // LiteLLM hands an unset location to vertexai.init, whose SDK takes us-central1; the cache key keeps the raw None
    let context = LegacyContext {
        project: deps.project,
        location: Some(deps.location.unwrap_or(DEFAULT_LOCATION)),
        model_is_language: false,
        services: deps.services,
    };
    let cached_client = cached.as_deref();
    if plan.acompletion {
        delegation::delegate(client, cached_client, plan, &context, access).await
    } else {
        let sync = sync::SyncDeps {
            context: &context,
            access,
            identifier: deps.identifier,
        };
        sync::execute(client, cached_client, plan, sync).await
    }
}

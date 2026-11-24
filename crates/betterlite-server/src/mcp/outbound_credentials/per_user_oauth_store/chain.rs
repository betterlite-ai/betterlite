use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use betterlite_mcp::oauth_identity_binding::port_types::{BindingDenied, RefreshOwnership};
use betterlite_mcp::server_record::MCPServer;

use super::super::authz_code_refresher::AuthorizationCodeRefresher;
use super::super::authz_code_refresher::ports::{
    CredentialPersist, ServerLookup, SystemEpochClock, TokenEndpointPost,
};
use super::super::cache_codec::OAuthTokenCacheCodec;
use super::super::cached_store::CachedOAuthTokenStore;
use super::super::dual_cache::{AsyncCache, DualCacheTokenCacheBackend};
use super::super::refresh_store::{InProcessRefreshCoordinator, RefreshingTokenStore};
use super::super::v2_store::{CredentialReader, V2PerUserTokenStore};
use super::super::{
    DEFAULT_EXPIRY_SKEW, DEFAULT_MAX_SIZE, InMemoryTokenCacheBackend, InvalidatableOAuthTokenStore,
    OAuthToken, OAuthTokenStore, PER_USER_TOKEN_KEY_PREFIX, RefreshCoordinator, RefreshStep,
    SystemWallClock, TokenCacheBackend, TokenCacheEntries, TokenKey, TokenRefresher,
    TokenStoreUnavailable,
};
use super::adapters::BoundCredentialReader;

// LiteLLM's _DEFAULT_TTL_SECONDS: a token with no declared expiry is cached this long.
const DEFAULT_TTL: Duration = Duration::from_secs(300);

pub struct RedisParts<Cache> {
    pub cache: Cache,
    pub codec: OAuthTokenCacheCodec,
    pub coordinator:
        Arc<dyn RefreshCoordinator<Output = Option<OAuthToken>, Error = TokenStoreUnavailable>>,
}

// LiteLLM's user_api_key_cache.redis_cache plus runtime_refresh_coordinator, both present or neither.
pub trait RedisChain {
    type Cache: AsyncCache;

    fn parts(&self) -> Option<RedisParts<Self::Cache>>;
}

pub struct BuiltStore {
    pub store: Arc<dyn InvalidatableOAuthTokenStore>,
    pub uses_redis: bool,
}

pub trait StoreBuilder: Send + Sync {
    fn build(&self) -> BuiltStore;
}

pub struct SharedRefreshCoordinator(
    pub Arc<dyn RefreshCoordinator<Output = Option<OAuthToken>, Error = TokenStoreUnavailable>>,
);

#[async_trait]
impl RefreshCoordinator for SharedRefreshCoordinator {
    type Output = Option<OAuthToken>;
    type Error = TokenStoreUnavailable;

    async fn run(
        &self,
        key: TokenKey,
        refresh: RefreshStep<'_, Self::Output, Self::Error>,
        reread: RefreshStep<'_, Self::Output, Self::Error>,
    ) -> Result<Self::Output, Self::Error> {
        self.0.run(key, refresh, reread).await
    }
}

// The binding port behind the chain, since B4715's trait is not implemented for Arc and the chain owns its collaborators.
pub struct SharedBinding<Binding>(pub Arc<Binding>);

impl<Binding> OauthBindingPort for SharedBinding<Binding>
where
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
{
    type Server = MCPServer;
    type Ownership = RefreshOwnership;
    type Denied = BindingDenied;

    async fn enforce(
        &self,
        request: &EnforceRequest<'_, MCPServer, RefreshOwnership>,
    ) -> Result<Option<String>, BindingDenied> {
        self.0.enforce(request).await
    }

    async fn proof_matches(
        &self,
        server: &MCPServer,
        user_id: &str,
        stored_proof: Option<&str>,
    ) -> bool {
        self.0.proof_matches(server, user_id, stored_proof).await
    }
}

pub struct PerUserStoreBuilder<Lookup, Binding, Reader, Persist, Post, Redis> {
    pub server_lookup: Arc<Lookup>,
    pub binding: Arc<Binding>,
    pub reader: Arc<Reader>,
    pub persist: Arc<Persist>,
    pub post: Arc<Post>,
    pub redis: Redis,
}

impl<Lookup, Binding, Reader, Persist, Post, Redis> StoreBuilder
    for PerUserStoreBuilder<Lookup, Binding, Reader, Persist, Post, Redis>
where
    Lookup: ServerLookup + 'static,
    Binding: OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>
        + 'static,
    Reader: CredentialReader + 'static,
    Persist: CredentialPersist + 'static,
    Post: TokenEndpointPost + 'static,
    Redis: RedisChain + Send + Sync + 'static,
    <Redis as RedisChain>::Cache: 'static,
{
    fn build(&self) -> BuiltStore {
        let reader = BoundCredentialReader {
            server_lookup: Arc::clone(&self.server_lookup),
            binding: Arc::clone(&self.binding),
            reader: Arc::clone(&self.reader),
        };
        let refresher = AuthorizationCodeRefresher {
            server_lookup: Arc::clone(&self.server_lookup),
            token_endpoint: Arc::clone(&self.post),
            persist: Arc::clone(&self.persist),
            binding: SharedBinding(Arc::clone(&self.binding)),
            clock: SystemEpochClock,
        };
        let inner = V2PerUserTokenStore { reader };
        match self.redis.parts() {
            Some(parts) => built_with_redis(inner, refresher, parts),
            None => built_locally(inner, refresher),
        }
    }
}

fn built_with_redis<Inner, Refresher, Cache>(
    inner: Inner,
    refresher: Refresher,
    parts: RedisParts<Cache>,
) -> BuiltStore
where
    Inner: OAuthTokenStore + 'static,
    Refresher: TokenRefresher + 'static,
    Cache: AsyncCache + 'static,
{
    let chain = RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: Arc::new(SystemWallClock),
        coordinator: SharedRefreshCoordinator(parts.coordinator),
    };
    let backend = DualCacheTokenCacheBackend {
        cache: parts.cache,
        codec: parts.codec,
        key_prefix: PER_USER_TOKEN_KEY_PREFIX,
    };
    BuiltStore {
        store: Arc::new(cached_store(chain, backend)),
        uses_redis: true,
    }
}

fn built_locally<Inner, Refresher>(inner: Inner, refresher: Refresher) -> BuiltStore
where
    Inner: OAuthTokenStore + 'static,
    Refresher: TokenRefresher + 'static,
{
    let chain = RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: Arc::new(SystemWallClock),
        coordinator:
            InProcessRefreshCoordinator::<Option<OAuthToken>, TokenStoreUnavailable>::default(),
    };
    let backend = InMemoryTokenCacheBackend {
        max_size: DEFAULT_MAX_SIZE,
        clock: Arc::new(SystemWallClock),
        entries: TokenCacheEntries::default(),
    };
    BuiltStore {
        store: Arc::new(cached_store(chain, backend)),
        uses_redis: false,
    }
}

fn cached_store<Inner, Backend>(
    inner: Inner,
    backend: Backend,
) -> CachedOAuthTokenStore<Inner, Backend>
where
    Inner: OAuthTokenStore,
    Backend: TokenCacheBackend,
{
    CachedOAuthTokenStore {
        inner,
        default_ttl: DEFAULT_TTL,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        backend,
        clock: Arc::new(SystemWallClock),
    }
}

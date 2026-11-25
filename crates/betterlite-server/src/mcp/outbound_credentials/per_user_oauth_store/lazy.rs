use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_db::oauth_token_resolution::OauthBindingPort;
use betterlite_mcp::oauth_identity_binding::port_types::{BindingDenied, RefreshOwnership};
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::server_record::binding::BindingMode;
use tokio::sync::{Mutex, Notify};

use super::super::authz_code_refresher::ports::ServerLookup;
use super::super::{
    InvalidatableOAuthTokenStore, OAuthToken, OAuthTokenStore, TokenStoreUnavailable,
};
use super::chain::StoreBuilder;

// LiteLLM's _redis_cache_is_available: asked again on every use, since Redis may connect after startup.
pub trait RedisAvailability: Send + Sync {
    fn is_available(&self) -> bool;
}

struct LazyState {
    store: Option<Arc<dyn InvalidatableOAuthTokenStore>>,
    uses_redis: bool,
}

// LiteLLM's finally: the in-flight count drops however the fetch or invalidate ends, a cancelled caller included.
struct LocalFetch<'store> {
    store: Arc<dyn InvalidatableOAuthTokenStore>,
    used_redis: bool,
    local_fetches: &'store AtomicUsize,
    notify: &'store Notify,
}

impl Drop for LocalFetch<'_> {
    fn drop(&mut self) {
        if !self.used_redis && self.local_fetches.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.notify.notify_waiters();
        }
    }
}

pub struct LazyPerUserOAuthTokenStore<Lookup, Binding, Builder, Availability> {
    server_lookup: Arc<Lookup>,
    binding: Arc<Binding>,
    store_builder: Builder,
    redis_available: Arc<Availability>,
    state: Mutex<LazyState>,
    local_fetches: AtomicUsize,
    notify: Notify,
}

#[async_trait]
impl<Lookup, Binding, Builder, Availability> OAuthTokenStore
    for LazyPerUserOAuthTokenStore<Lookup, Binding, Builder, Availability>
where
    Lookup: ServerLookup,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Builder: StoreBuilder,
    Availability: RedisAvailability,
{
    async fn fetch(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        let Some(token) = self.fetch_token(user_id, server_id).await? else {
            return Ok(None);
        };
        let Some(server) = self.server_lookup.server(server_id) else {
            return Ok(Some(token));
        };
        let Some(binding) = &server.fields.oauth_identity_binding else {
            return Ok(Some(token));
        };
        if binding.mode != BindingMode::Enforce {
            return Ok(Some(token));
        }
        let matches = self
            .binding
            .proof_matches(&server, user_id, token.identity_binding_proof.as_deref())
            .await;
        if matches {
            return Ok(Some(token));
        }
        self.invalidate(user_id, server_id).await;
        Ok(None)
    }
}

#[async_trait]
impl<Lookup, Binding, Builder, Availability> InvalidatableOAuthTokenStore
    for LazyPerUserOAuthTokenStore<Lookup, Binding, Builder, Availability>
where
    Lookup: ServerLookup,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Builder: StoreBuilder,
    Availability: RedisAvailability,
{
    async fn invalidate(&self, user_id: &str, server_id: &str) {
        if let Some(store) = self.redis_store().await {
            store.invalidate(user_id, server_id).await;
            return;
        }
        let local = self.store_for_fetch().await;
        local.store.invalidate(user_id, server_id).await;
        drop(local);
    }
}

impl<Lookup, Binding, Builder, Availability>
    LazyPerUserOAuthTokenStore<Lookup, Binding, Builder, Availability>
where
    Lookup: ServerLookup,
    Binding:
        OauthBindingPort<Server = MCPServer, Ownership = RefreshOwnership, Denied = BindingDenied>,
    Builder: StoreBuilder,
    Availability: RedisAvailability,
{
    #[must_use]
    pub fn new(
        server_lookup: Arc<Lookup>,
        binding: Arc<Binding>,
        store_builder: Builder,
        redis_available: Arc<Availability>,
    ) -> Self {
        Self {
            server_lookup,
            binding,
            store_builder,
            redis_available,
            state: Mutex::new(LazyState {
                store: None,
                uses_redis: false,
            }),
            local_fetches: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }

    async fn fetch_token(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        if let Some(store) = self.redis_store().await {
            return store.fetch(user_id, server_id).await;
        }
        let local = self.store_for_fetch().await;
        let result = local.store.fetch(user_id, server_id).await;
        drop(local);
        result
    }

    pub(super) async fn redis_store(&self) -> Option<Arc<dyn InvalidatableOAuthTokenStore>> {
        let state = self.state.lock().await;
        if state.uses_redis {
            state.store.clone()
        } else {
            None
        }
    }

    // An in-process chain is replaced after its in-flight fetches finish, so one refresh token is redeemed once.
    async fn store_for_fetch(&self) -> LocalFetch<'_> {
        loop {
            let mut notified = Box::pin(self.notify.notified());
            notified.as_mut().enable();
            let mut state = self.state.lock().await;
            if state.store.is_some()
                && !state.uses_redis
                && self.redis_available.is_available()
                && self.local_fetches.load(Ordering::SeqCst) > 0
            {
                drop(state);
                notified.await;
                continue;
            }
            if state.store.is_none() || (!state.uses_redis && self.redis_available.is_available()) {
                let built = self.store_builder.build();
                state.uses_redis = built.uses_redis;
                state.store = Some(built.store);
            }
            let uses_redis = state.uses_redis;
            let Some(store) = state.store.as_ref().map(Arc::clone) else {
                continue;
            };
            if !uses_redis {
                self.local_fetches.fetch_add(1, Ordering::SeqCst);
            }
            drop(state);
            return LocalFetch {
                store,
                used_redis: uses_redis,
                local_fetches: &self.local_fetches,
                notify: &self.notify,
            };
        }
    }
}

mod tests;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::watch;

use super::{
    OAuthToken, OAuthTokenStore, RefreshCoordinator, RefreshStep, TokenKey, TokenRefresher,
    TokenStoreUnavailable, WallClock,
};

pub type Inflight<Output, Error> =
    Mutex<HashMap<TokenKey, watch::Sender<Option<Result<Output, Error>>>>>;

pub struct InProcessRefreshCoordinator<Output, Error> {
    pub inflight: Inflight<Output, Error>,
}

impl<Output, Error> Default for InProcessRefreshCoordinator<Output, Error> {
    fn default() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }
}

enum Role<Output, Error> {
    Leader(watch::Sender<Option<Result<Output, Error>>>),
    Follower(watch::Receiver<Option<Result<Output, Error>>>),
}

#[async_trait]
impl<Output, Error> RefreshCoordinator for InProcessRefreshCoordinator<Output, Error>
where
    Output: Clone + Send + Sync + 'static,
    Error: Clone + Send + Sync + 'static,
{
    type Output = Output;
    type Error = Error;

    async fn run(
        &self,
        key: TokenKey,
        refresh: RefreshStep<'_, Output, Error>,
        _reread: RefreshStep<'_, Output, Error>,
    ) -> Result<Output, Error> {
        loop {
            let role = {
                let mut inflight = self.inflight.lock().unwrap_or_else(PoisonError::into_inner);
                match inflight.entry(key.clone()) {
                    Entry::Occupied(entry) => Role::Follower(entry.get().subscribe()),
                    Entry::Vacant(entry) => {
                        let (sender, _receiver) = watch::channel(None);
                        entry.insert(sender.clone());
                        Role::Leader(sender)
                    }
                }
            };
            match role {
                Role::Leader(sender) => {
                    let result = refresh().await;
                    // Removed before the send, so a caller arriving now starts a fresh refresh.
                    drop(
                        self.inflight
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&key),
                    );
                    drop(sender.send_replace(Some(result.clone())));
                    return result;
                }
                Role::Follower(mut receiver) => {
                    let current: Option<Result<Output, Error>> = (*receiver.borrow()).clone();
                    if let Some(result) = current {
                        return result;
                    }
                    if receiver.changed().await.is_err() {
                        // The leader's call ended without a result, so this caller takes over.
                        continue;
                    }
                    let published: Option<Result<Output, Error>> = (*receiver.borrow()).clone();
                    if let Some(result) = published {
                        return result;
                    }
                }
            }
        }
    }
}

pub struct RefreshingTokenStore<Inner, Refresher, Coordinator> {
    pub inner: Inner,
    pub refresher: Refresher,
    pub expiry_skew: Duration,
    pub clock: Arc<dyn WallClock>,
    pub coordinator: Coordinator,
}

impl<Inner, Refresher, Coordinator> RefreshingTokenStore<Inner, Refresher, Coordinator>
where
    Inner: OAuthTokenStore,
    Refresher: TokenRefresher,
    Coordinator: Send + Sync,
{
    async fn refresh_latest(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        let Some(latest) = self.inner.fetch(user_id, server_id).await? else {
            return Ok(None);
        };
        if latest.is_expired(self.clock.now(), self.expiry_skew) {
            return self.refresher.refresh(user_id, server_id, &latest).await;
        }
        Ok(Some(latest))
    }
}

#[async_trait]
impl<Inner, Refresher, Coordinator> OAuthTokenStore
    for RefreshingTokenStore<Inner, Refresher, Coordinator>
where
    Inner: OAuthTokenStore,
    Refresher: TokenRefresher,
    Coordinator: RefreshCoordinator<Output = Option<OAuthToken>, Error = TokenStoreUnavailable>,
{
    async fn fetch(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        let Some(token) = self.inner.fetch(user_id, server_id).await? else {
            return Ok(None);
        };
        if !token.is_expired(self.clock.now(), self.expiry_skew) {
            return Ok(Some(token));
        }
        let key = TokenKey {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        };
        let refresh: RefreshStep<'_, Option<OAuthToken>, TokenStoreUnavailable> =
            Box::new(|| Box::pin(self.refresh_latest(user_id, server_id)));
        let reread: RefreshStep<'_, Option<OAuthToken>, TokenStoreUnavailable> = Box::new(|| {
            Box::pin(async move {
                let latest = self.inner.fetch(user_id, server_id).await?;
                Ok(latest
                    .filter(|candidate| !candidate.is_expired(self.clock.now(), self.expiry_skew)))
            })
        });
        self.coordinator.run(key, refresh, reread).await
    }
}

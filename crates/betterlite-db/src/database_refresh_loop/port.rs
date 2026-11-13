use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use secrecy::{ExposeSecret, SecretString};

use crate::database_refresh::access::{RefreshFuture, RefreshTrigger};
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::generation::PoolGeneration;
use crate::database_refresh::handle::PoolProvider;
use crate::database_refresh::replacement::{self, ReplacementError};
use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::expiry::{ExpiryError, TokenAuthKind};
use crate::database_url_policy::url_codec::UrlSplitError;

use super::clock::RefreshClock;
use super::endpoint::{WriterEndpoint, WriterEndpointError};
use super::minting::{TokenMinter, assemble_refresh_url};
use super::schedule::{
    self, RefreshQuery, RefreshSchedule, is_token_expired, token_refresh_not_needed,
};

pub type RefreshGeneration<P, M, C> = PoolGeneration<P, RefreshPort<P, M, C>, C>;

type RefreshLink<P, M, C> = Mutex<Option<Weak<RefreshGeneration<P, M, C>>>>;

type Attached<P, M, C> = Result<Arc<RefreshGeneration<P, M, C>>, RefreshPortError<P, M, C>>;

pub type RefreshPortError<P, M, C> =
    RefreshError<<P as PoolProvider>::Error, C, <M as TokenMinter>::Error>;

pub struct RefreshPortConfig<Minter> {
    pub auth_kind: Option<TokenAuthKind>,
    pub writer_endpoint: WriterEndpoint,
    pub iam_endpoint: Option<IAMEndpoint>,
    pub url: DatabaseUrlSlot,
    pub clock: Arc<dyn RefreshClock>,
    pub minter: Minter,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError<PoolError, CallbackError, MintError>
where
    PoolError: Error + Send + Sync + 'static,
    CallbackError: Error + Send + Sync + 'static,
    MintError: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Endpoint(#[from] WriterEndpointError),
    #[error("the database token could not be minted")]
    Mint {
        #[source]
        source: MintError,
    },
    #[error("the refreshed database URL could not be assembled")]
    Url {
        #[source]
        source: UrlSplitError,
    },
    #[error("the database token expiration could not be read")]
    Expiry {
        #[source]
        source: ExpiryError,
    },
    #[error("the database pool replacement failed")]
    Replace {
        #[source]
        source: ReplacementError<PoolError, CallbackError>,
    },
    #[error("the database refresh port is not attached to a pool generation")]
    Detached,
}

pub struct RefreshPort<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    minter: M,
    auth_kind: Option<TokenAuthKind>,
    writer_endpoint: WriterEndpoint,
    iam_endpoint: Option<IAMEndpoint>,
    url: DatabaseUrlSlot,
    clock: Arc<dyn RefreshClock>,
    generation: RefreshLink<P, M, C>,
}

// The replacement publishes the candidate before and independently of this snapshot.
struct UrlRestore<'slot> {
    slot: &'slot DatabaseUrlSlot,
    previous: Option<SecretString>,
    armed: bool,
}

impl Drop for UrlRestore<'_> {
    fn drop(&mut self) {
        if self.armed {
            drop(self.slot.publish(self.previous.take()));
        }
    }
}

impl<P, M, C> RefreshPort<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    #[must_use]
    pub fn new(config: RefreshPortConfig<M>) -> Self {
        Self {
            minter: config.minter,
            auth_kind: config.auth_kind,
            writer_endpoint: config.writer_endpoint,
            iam_endpoint: config.iam_endpoint,
            url: config.url,
            clock: config.clock,
            generation: Mutex::new(None),
        }
    }

    pub fn attach(&self, generation: &Arc<RefreshGeneration<P, M, C>>) {
        let weak = Arc::downgrade(generation);
        *self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(weak);
    }

    fn attached(&self) -> Attached<P, M, C> {
        let generation = self
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        generation
            .and_then(|weak| weak.upgrade())
            .ok_or(RefreshError::Detached)
    }

    pub(crate) fn schedule(&self) -> Result<RefreshSchedule, ExpiryError> {
        let current = self.url.snapshot();
        let query = RefreshQuery {
            kind: self.auth_kind,
            token_url: current.as_ref().map(ExposeSecret::expose_secret),
            now: self.clock.now().naive_utc(),
        };
        schedule::seconds_until_refresh(&query)
    }

    fn mint_and_publish(
        &self,
    ) -> Result<Option<SecretString>, RefreshError<P::Error, C, M::Error>> {
        if self.auth_kind.is_none() {
            return Ok(None);
        }
        let endpoint = match &self.iam_endpoint {
            Some(endpoint) => endpoint.clone(),
            None => self
                .writer_endpoint
                .to_iam_endpoint()
                .map_err(RefreshError::Endpoint)?,
        };
        let Some(token) = self
            .minter
            .token(&endpoint)
            .map_err(|source| RefreshError::Mint { source })?
        else {
            return Ok(None);
        };
        let previous = self.url.snapshot();
        let url = assemble_refresh_url(&endpoint, &token, previous.as_ref())
            .map_err(|source| RefreshError::Url { source })?;
        drop(self.url.publish(Some(url.clone())));
        Ok(Some(url))
    }

    async fn safe_refresh(&self) -> Result<(), RefreshPortError<P, M, C>> {
        let generation = self.attached()?;
        let _reconnection = generation.replacement_lock.lock().await;
        let prefix = generation.options.log_prefix.as_str();
        let label = generation.token_label();
        let current = self.url.snapshot();
        let query = RefreshQuery {
            kind: self.auth_kind,
            token_url: current.as_ref().map(ExposeSecret::expose_secret),
            now: self.clock.now().naive_utc(),
        };
        let fresh =
            token_refresh_not_needed(&query).map_err(|source| RefreshError::Expiry { source })?;
        if fresh {
            tracing::debug!("{prefix}{label} still fresh; skipping redundant refresh.");
            return Ok(());
        }
        let mut restore = UrlRestore {
            slot: &self.url,
            previous: current,
            armed: true,
        };
        match self.mint_and_publish()? {
            Some(url) => {
                replacement::replace_locked(&generation, &url, None)
                    .await
                    .map_err(|source| RefreshError::Replace { source })?;
                generation.record_refresh(self.clock.now());
                tracing::info!("{prefix}{label} refreshed successfully.");
            }
            None => {
                tracing::error!("{prefix}Failed to generate new {label} during proactive refresh");
            }
        }
        restore.armed = false;
        Ok(())
    }
}

impl<P, M, C> RefreshTrigger for RefreshPort<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    type Error = RefreshError<P::Error, C, M::Error>;

    fn token_expired(&self) -> bool {
        let current = self.url.snapshot();
        let query = RefreshQuery {
            kind: self.auth_kind,
            token_url: current.as_ref().map(ExposeSecret::expose_secret),
            now: self.clock.now().naive_utc(),
        };
        // the port reports a bool, so an expiration outside the timestamp range counts as expired
        is_token_expired(&query).unwrap_or(true)
    }

    fn refresh(&self) -> RefreshFuture<'_, Self::Error> {
        Box::pin(self.safe_refresh())
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Self::Error> {
        self.mint_and_publish()
    }
}

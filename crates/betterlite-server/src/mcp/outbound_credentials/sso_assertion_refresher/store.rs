mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::sso_assertion_rows::AssertionRows;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, assertion_expired};
use betterlite_mcp::sso_assertions::read::{
    AssertionStoreUnavailable, SsoAssertionStore, fetch_uncached,
};
use betterlite_mcp::sso_assertions::store::DbSsoAssertionStore;
use chrono::TimeDelta;
use thiserror::Error;

use super::client_config::SsoClientSource;
use super::failure::RefreshFailureKind;
use super::refresher::{AssertionAccess, AssertionRefresher};
use super::transport::RefreshTransport;
use crate::mcp::outbound_credentials::{RefreshCoordinator, RefreshStep, TokenKey, WallClock};

pub const SINGLE_FLIGHT_KEY: &str = "sso_identity_assertion";
pub const DEFAULT_EXPIRY_SKEW: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{detail}")]
pub struct RenewalUnavailable {
    pub detail: String,
}

impl From<AssertionStoreUnavailable> for RenewalUnavailable {
    fn from(unavailable: AssertionStoreUnavailable) -> Self {
        Self {
            detail: unavailable.cause.to_string(),
        }
    }
}

pub type RenewalCoordinator = Arc<dyn RefreshCoordinator<Output = (), Error = RenewalUnavailable>>;

pub trait CoordinatorFactory: Send + Sync {
    fn coordinator(&self) -> Option<RenewalCoordinator>;
}

pub struct RefreshingAssertionStore<I, T, C, A, F> {
    pub inner: I,
    pub refresher: AssertionRefresher<T, C, A>,
    pub access: Arc<A>,
    pub factory: F,
    pub in_process: RenewalCoordinator,
    pub distributed: Mutex<Option<RenewalCoordinator>>,
    pub skew: Duration,
    pub clock: Arc<dyn WallClock>,
}

impl<I, T, C, A, F> RefreshingAssertionStore<I, T, C, A, F>
where
    I: SsoAssertionStore + Send + Sync,
    T: RefreshTransport,
    C: SsoClientSource,
    A: AssertionAccess,
    F: CoordinatorFactory,
{
    // Built on first use and kept, because the Redis client is not wired at start; asked again while it answers None.
    fn coordinator(&self) -> RenewalCoordinator {
        let mut distributed = self
            .distributed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if distributed.is_none() {
            *distributed = self.factory.coordinator();
        }
        let chosen = distributed.as_ref().cloned();
        drop(distributed);
        chosen.unwrap_or_else(|| Arc::clone(&self.in_process))
    }

    fn expiring(&self, assertion: Option<&SsoIdentityAssertion>) -> bool {
        let Some(stored) = assertion else {
            return false;
        };
        let Ok(skew) = TimeDelta::from_std(self.skew) else {
            return true;
        };
        self.clock
            .now()
            .checked_add_signed(skew)
            .is_none_or(|deadline| assertion_expired(stored, deadline))
    }

    // The elected renewal, judged from a fresh read so a rotation another replica just landed is never redeemed again.
    async fn renew(&self, user_id: &str) -> Result<(), RenewalUnavailable> {
        let stored = self.access.read(user_id).await?;
        let Some(assertion) = stored.filter(|assertion| self.expiring(Some(assertion))) else {
            return Ok(());
        };
        match self.refresher.refresh(user_id, &assertion).await {
            Ok(_renewed) => Ok(()),
            Err(failure) => match failure.kind {
                RefreshFailureKind::Rejected => Ok(()),
                RefreshFailureKind::Unavailable => Err(RenewalUnavailable {
                    detail: failure.detail,
                }),
            },
        }
    }

    // A loser cannot tell a refusal from an unrecorded renewal without risking token replay, so it answers retryable.
    async fn reread_renewed(&self, user_id: &str) -> Result<(), RenewalUnavailable> {
        if !self.expiring(self.access.read(user_id).await?.as_ref()) {
            return Ok(());
        }
        Err(RenewalUnavailable {
            detail: format!(
                "the IdP identity assertion for user_id={user_id} was being renewed by another replica and is not yet current; retry shortly"
            ),
        })
    }
}

impl<I, T, C, A, F> SsoAssertionStore for RefreshingAssertionStore<I, T, C, A, F>
where
    I: SsoAssertionStore + Send + Sync,
    T: RefreshTransport,
    C: SsoClientSource,
    A: AssertionAccess,
    F: CoordinatorFactory,
{
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        let assertion = self.inner.fetch(user_id).await?;
        if !self.expiring(assertion.as_ref()) {
            return Ok(assertion);
        }
        let key = TokenKey {
            user_id: user_id.to_owned(),
            server_id: SINGLE_FLIGHT_KEY.to_owned(),
        };
        let refresh: RefreshStep<'_, (), RenewalUnavailable> =
            Box::new(|| Box::pin(self.renew(user_id)));
        let reread: RefreshStep<'_, (), RenewalUnavailable> =
            Box::new(|| Box::pin(self.reread_renewed(user_id)));
        self.coordinator()
            .run(key, refresh, reread)
            .await
            .map_err(|unavailable| AssertionStoreUnavailable {
                cause: Box::new(unavailable),
            })?;
        self.access.read(user_id).await
    }
}

#[async_trait]
impl<R> AssertionAccess for DbSsoAssertionStore<R>
where
    R: AssertionRows + Send + Sync,
{
    async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        fetch_uncached(self, user_id).await
    }

    async fn write(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), AssertionStoreUnavailable> {
        self.persist(user_id, assertion)
            .await
            .map_err(|source| AssertionStoreUnavailable {
                cause: Box::new(source),
            })
    }
}

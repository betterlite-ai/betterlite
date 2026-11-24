use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};
use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::client_config::{
    SsoClientConfig, SsoClientSource, sso_client_config,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::failure::{
    RefreshFailure, RefreshFailureKind,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::refresher::{
    AssertionAccess, AssertionRefresher,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::store::{
    CoordinatorFactory, DEFAULT_EXPIRY_SKEW, RefreshingAssertionStore, RenewalCoordinator,
    RenewalUnavailable,
};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::transport::RefreshTransport;
use betterlite_server::mcp::outbound_credentials::{
    RefreshCoordinator, RefreshStep, TokenKey, WallClock,
};
use chrono::{DateTime, Utc};

use betterlite_oracle_helper_api::error::HelperError;

use super::support::{AccessFailure, OutcomeSpec, TransportAnswer, scripted, value_error};

const READ_FAILURE: &str = "the scripted row read failed";
const WRITE_FAILURE: &str = "the scripted row write failed";

fn unavailable(message: &str) -> AssertionStoreUnavailable {
    AssertionStoreUnavailable {
        cause: Box::new(io::Error::other(message.to_owned())),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecordedCall {
    pub(crate) url: String,
    pub(crate) form: BTreeMap<String, String>,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct Harness {
    pub(crate) inner: Arc<Mutex<BTreeMap<String, SsoIdentityAssertion>>>,
    pub(crate) fresh: Arc<Mutex<BTreeMap<String, SsoIdentityAssertion>>>,
    pub(crate) writes: Arc<Mutex<Vec<(String, SsoIdentityAssertion)>>>,
    pub(crate) vars: BTreeMap<String, String>,
    pub(crate) outcomes: Arc<Mutex<VecDeque<TransportAnswer>>>,
    pub(crate) calls: Arc<Mutex<Vec<RecordedCall>>>,
    pub(crate) failure: AccessFailure,
}

impl Harness {
    pub(crate) fn new(
        inner: BTreeMap<String, SsoIdentityAssertion>,
        fresh: BTreeMap<String, SsoIdentityAssertion>,
        failure: AccessFailure,
        vars: &BTreeMap<String, String>,
        outcome: &OutcomeSpec,
    ) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            fresh: Arc::new(Mutex::new(fresh)),
            writes: Arc::new(Mutex::new(Vec::new())),
            vars: vars.clone(),
            outcomes: Arc::new(Mutex::new(VecDeque::from([scripted(outcome)?]))),
            calls: Arc::new(Mutex::new(Vec::new())),
            failure,
        })
    }

    pub(crate) fn refresher(&self) -> AssertionRefresher<Self, Self, Self> {
        AssertionRefresher {
            transport: self.clone(),
            client_source: self.clone(),
            access: Arc::new(self.clone()),
        }
    }

    pub(crate) fn store(
        &self,
        loser: bool,
        now_ms: i64,
    ) -> Result<RefreshingAssertionStore<Self, Self, Self, Self, FixedFactory>, Box<HelperError>>
    {
        Ok(RefreshingAssertionStore {
            inner: self.clone(),
            refresher: self.refresher(),
            access: Arc::new(self.clone()),
            factory: FixedFactory { loser },
            in_process: Arc::new(InProcessRefreshCoordinator::default()),
            distributed: Mutex::new(None),
            skew: DEFAULT_EXPIRY_SKEW,
            clock: clock(now_ms)?,
        })
    }
}

impl SsoAssertionStore for Harness {
    fn fetch(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>> + Send
    {
        let stored = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(stored.get(user_id).cloned()))
    }
}

impl SsoClientSource for Harness {
    fn client(&self) -> Option<SsoClientConfig> {
        sso_client_config(&self.vars)
    }
}

#[async_trait]
impl RefreshTransport for Harness {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> TransportAnswer {
        let call = RecordedCall {
            url: url.to_owned(),
            form: form.clone(),
            headers: headers.clone(),
        };
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
        drop(calls);
        let mut outcomes = self.outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        outcomes.pop_front().unwrap_or_else(|| {
            Err(RefreshFailure {
                kind: RefreshFailureKind::Unavailable,
                detail: "the case scripted no outcome".to_owned(),
            })
        })
    }
}

#[async_trait]
impl AssertionAccess for Harness {
    async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        match self.failure {
            AccessFailure::Read => Err(unavailable(READ_FAILURE)),
            AccessFailure::None | AccessFailure::Write => {
                let stored = self.fresh.lock().unwrap_or_else(PoisonError::into_inner);
                Ok(stored.get(user_id).cloned())
            }
        }
    }

    async fn write(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), AssertionStoreUnavailable> {
        if matches!(self.failure, AccessFailure::Write) {
            return Err(unavailable(WRITE_FAILURE));
        }
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.push((user_id.to_owned(), assertion.clone()));
        drop(writes);
        let mut fresh = self.fresh.lock().unwrap_or_else(PoisonError::into_inner);
        fresh.insert(user_id.to_owned(), assertion.clone());
        drop(fresh);
        Ok(())
    }
}

struct FixedClock(DateTime<Utc>);

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn clock(now_ms: i64) -> Result<Arc<dyn WallClock>, Box<HelperError>> {
    let now = DateTime::from_timestamp_millis(now_ms)
        .ok_or_else(|| value_error(format!("invalid now_ms: {now_ms}")))?;
    Ok(Arc::new(FixedClock(now)))
}

struct LoserCoordinator;

#[async_trait]
impl RefreshCoordinator for LoserCoordinator {
    type Output = ();
    type Error = RenewalUnavailable;

    async fn run(
        &self,
        _key: TokenKey,
        _refresh: RefreshStep<'_, (), RenewalUnavailable>,
        reread: RefreshStep<'_, (), RenewalUnavailable>,
    ) -> Result<(), RenewalUnavailable> {
        reread().await
    }
}

pub(crate) struct FixedFactory {
    pub(crate) loser: bool,
}

impl CoordinatorFactory for FixedFactory {
    fn coordinator(&self) -> Option<RenewalCoordinator> {
        if self.loser {
            return Some(Arc::new(LoserCoordinator));
        }
        None
    }
}

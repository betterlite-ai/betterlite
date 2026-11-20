#![cfg(test)]

use std::str;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::http::Method;
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::CredentialRef;
use betterlite_auth::auth_identity::records::NetworkContext;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use betterlite_db::migration_ledger::baseline::PACKAGED_CATALOGS;
use betterlite_db::migration_ledger::baseline::catalog_statements;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::DateTime;
use chrono::TimeZone as _;
use chrono::Utc;
use clap::Parser as _;
use deadpool_postgres::Manager;
use deadpool_postgres::Pool;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio_postgres::NoTls;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::user_agent_analytics::error::AnalyticsError;
use crate::user_agent_analytics::query::TagFilter;
use crate::user_agent_analytics::routes;
use crate::user_agent_analytics::rows::ActiveUsersReport;
use crate::user_agent_analytics::rows::ActiveUsersRow;
use crate::user_agent_analytics::rows::DistinctTagRow;
use crate::user_agent_analytics::rows::TagSpendRecord;
use crate::user_agent_analytics::rows::TagSummaryRow;
use crate::user_agent_analytics::rows::TokenOwnerRow;
use crate::user_agent_analytics::rows::UserEmailRow;
use crate::user_agent_analytics::state::UserAgentAnalyticsState;
use crate::user_agent_analytics::store::PostgresAnalyticsStore;
use crate::user_agent_analytics::store::UserAgentAnalyticsStore;

pub(crate) fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("the fixed instant")
}

pub(super) fn now() -> DateTime<Utc> {
    at(2026, 1, 10, 12, 0)
}

struct FixedClock(DateTime<Utc>);

impl GatewayRequestClock for FixedClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(super) fn fixed_clock(instant: DateTime<Utc>) -> Arc<dyn GatewayRequestClock> {
    Arc::new(FixedClock(instant))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveUsersCall {
    pub(crate) report: ActiveUsersReport,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) filter: Option<TagFilter>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RangeCall {
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) filter: Option<TagFilter>,
}

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(super) distinct: Vec<DistinctTagRow>,
    pub(super) active: Vec<ActiveUsersRow>,
    summary: Vec<TagSummaryRow>,
    records: Vec<TagSpendRecord>,
    owners: Vec<TokenOwnerRow>,
    users: Vec<UserEmailRow>,
    pub(super) failure: Mutex<Option<AnalyticsError>>,
    pub(super) calls: Mutex<Vec<ActiveUsersCall>>,
    summary_calls: Mutex<Vec<RangeCall>>,
    record_calls: Mutex<Vec<RangeCall>>,
    token_calls: Mutex<Vec<Vec<String>>>,
    user_calls: Mutex<Vec<Vec<String>>>,
    pub(super) distinct_calls: AtomicUsize,
}

impl FakeStore {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn failing() -> Self {
        Self {
            failure: Mutex::new(Some(AnalyticsError::Database(DbError::NoDbConnected))),
            ..Self::empty()
        }
    }

    pub(crate) fn with_active(mut self, rows: Vec<ActiveUsersRow>) -> Self {
        self.active = rows;
        self
    }

    pub(crate) fn with_summary(mut self, rows: Vec<TagSummaryRow>) -> Self {
        self.summary = rows;
        self
    }

    pub(crate) fn with_records(mut self, records: Vec<TagSpendRecord>) -> Self {
        self.records = records;
        self
    }

    pub(crate) fn with_owners(mut self, owners: Vec<TokenOwnerRow>) -> Self {
        self.owners = owners;
        self
    }

    pub(crate) fn with_users(mut self, users: Vec<UserEmailRow>) -> Self {
        self.users = users;
        self
    }

    pub(super) fn calls(&self) -> Vec<ActiveUsersCall> {
        self.calls.lock().expect("the recorded calls").clone()
    }

    pub(crate) fn only_call(&self) -> ActiveUsersCall {
        let mut calls = self.calls();
        assert_eq!(calls.len(), 1, "the store records exactly one call");
        calls.remove(0)
    }

    pub(crate) fn only_summary(&self) -> RangeCall {
        only(&self.summary_calls)
    }

    pub(crate) fn only_records(&self) -> RangeCall {
        only(&self.record_calls)
    }

    pub(crate) fn token_calls(&self) -> Vec<Vec<String>> {
        self.token_calls.lock().expect("the key lookups").clone()
    }

    pub(crate) fn user_calls(&self) -> Vec<Vec<String>> {
        self.user_calls.lock().expect("the user lookups").clone()
    }

    pub(super) fn distinct_calls(&self) -> usize {
        self.distinct_calls.load(Ordering::Relaxed)
    }

    fn take_failure(&self) -> Option<AnalyticsError> {
        self.failure.lock().expect("the recorded failure").take()
    }
}

fn only<Record: Clone>(calls: &Mutex<Vec<Record>>) -> Record {
    let mut records = calls.lock().expect("the recorded calls").clone();
    assert_eq!(records.len(), 1, "the store records exactly one call");
    records.remove(0)
}

#[async_trait]
impl UserAgentAnalyticsStore for FakeStore {
    async fn distinct_tags(&self) -> Result<Vec<DistinctTagRow>, AnalyticsError> {
        self.distinct_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.distinct.clone())
    }

    async fn active_users(
        &self,
        report: ActiveUsersReport,
        start_date: &str,
        end_date: &str,
        filter: Option<&TagFilter>,
    ) -> Result<Vec<ActiveUsersRow>, AnalyticsError> {
        self.calls
            .lock()
            .expect("the recorded calls")
            .push(ActiveUsersCall {
                report,
                start_date: start_date.to_owned(),
                end_date: end_date.to_owned(),
                filter: filter.cloned(),
            });
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.active.clone())
    }

    async fn tag_summary(
        &self,
        start_date: &str,
        end_date: &str,
        filter: Option<&TagFilter>,
    ) -> Result<Vec<TagSummaryRow>, AnalyticsError> {
        self.summary_calls
            .lock()
            .expect("the recorded calls")
            .push(RangeCall {
                start_date: start_date.to_owned(),
                end_date: end_date.to_owned(),
                filter: filter.cloned(),
            });
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.summary.clone())
    }

    async fn tag_records(
        &self,
        start_date: &str,
        end_date: &str,
        filter: Option<&TagFilter>,
    ) -> Result<Vec<TagSpendRecord>, AnalyticsError> {
        self.record_calls
            .lock()
            .expect("the recorded calls")
            .push(RangeCall {
                start_date: start_date.to_owned(),
                end_date: end_date.to_owned(),
                filter: filter.cloned(),
            });
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.records.clone())
    }

    async fn token_owners(&self, tokens: &[String]) -> Result<Vec<TokenOwnerRow>, AnalyticsError> {
        self.token_calls
            .lock()
            .expect("the key lookups")
            .push(tokens.to_vec());
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.owners.clone())
    }

    async fn user_emails(&self, user_ids: &[String]) -> Result<Vec<UserEmailRow>, AnalyticsError> {
        self.user_calls
            .lock()
            .expect("the user lookups")
            .push(user_ids.to_vec());
        if let Some(error) = self.take_failure() {
            return Err(error);
        }
        Ok(self.users.clone())
    }
}

pub(crate) fn app(store: Option<Arc<FakeStore>>) -> Router {
    app_at(store, now())
}

fn erased(store: Arc<FakeStore>) -> Arc<dyn UserAgentAnalyticsStore> {
    store
}

pub(crate) fn app_at(store: Option<Arc<FakeStore>>, instant: DateTime<Utc>) -> Router {
    routes(UserAgentAnalyticsState {
        store: store.map(erased),
        clock: fixed_clock(instant),
    })
}

pub(crate) fn postgres_app(pool: &Pool) -> Router {
    let store: Arc<dyn UserAgentAnalyticsStore> = Arc::new(postgres_store(pool));
    routes(UserAgentAnalyticsState {
        store: Some(store),
        clock: fixed_clock(now()),
    })
}

pub(super) fn postgres_store(pool: &Pool) -> PostgresAnalyticsStore {
    PostgresAnalyticsStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

pub(super) fn pool_less_store() -> PostgresAnalyticsStore {
    PostgresAnalyticsStore {
        store: PostgresStore { pool: None },
    }
}

pub(crate) fn caller() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "virtual-key".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(crate) async fn get(app: Router, identity: Option<&AuthContext>, uri: &str) -> Reply {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds");
    if let Some(context) = identity {
        request.extensions_mut().insert(context.clone());
    }
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body");
    Reply {
        status,
        body: body.to_vec(),
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("the response is JSON")
    }

    pub(crate) fn text(&self) -> String {
        str::from_utf8(&self.body)
            .expect("the response is UTF-8")
            .to_owned()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DetailBody {
    pub(crate) detail: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NestedBody {
    pub(crate) detail: NestedDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NestedDetail {
    pub(crate) error: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorEnvelope {
    pub(super) error: ErrorFields,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorFields {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) error_type: String,
    pub(super) param: String,
    pub(super) code: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct DistinctBody {
    pub(super) results: Vec<DistinctTagBody>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct DistinctTagBody {
    pub(super) tag: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ActiveUsersBody {
    pub(super) results: Vec<ActiveUserBody>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ActiveUserBody {
    pub(super) tag: String,
    pub(super) active_users: i64,
    pub(super) date: String,
    pub(super) period_start: Option<String>,
    pub(super) period_end: Option<String>,
}

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["user-agent-analytics-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the packaged catalogs open");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the test database is provisioned");
    let manager = Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses"),
        NoTls,
    );
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the pool builds");
    (fixture, pool)
}

pub(crate) async fn seed(pool: &Pool, statements: &str) {
    pool.get()
        .await
        .expect("a pooled client")
        .batch_execute(statements)
        .await
        .expect("the seeded rows");
}

pub(crate) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}

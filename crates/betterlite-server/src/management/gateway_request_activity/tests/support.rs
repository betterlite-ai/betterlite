#![cfg(test)]

use std::str;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::CredentialRef;
use betterlite_auth::auth_identity::records::NetworkContext;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use betterlite_db::migration_ledger::baseline::PACKAGED_CATALOGS;
use betterlite_db::migration_ledger::baseline::catalog_statements;
use betterlite_db::postgres_store::diagnostics::PoolFailure;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::DateTime;
use chrono::TimeZone as _;
use chrono::Utc;
use clap::Parser as _;
use deadpool_postgres::Manager;
use deadpool_postgres::Pool;
use deadpool_postgres::PoolError;
use deadpool_postgres::TimeoutType;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio_postgres::NoTls;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::management::gateway_request_activity::GatewayActivityState;
use crate::management::gateway_request_activity::router;
use crate::management::gateway_request_activity::store::GatewayActivityRow;
use crate::management::gateway_request_activity::store::GatewayActivityStore;
use crate::management::gateway_request_activity::store::PostgresActivityStore;

struct FixedClock(DateTime<Utc>);

impl GatewayRequestClock for FixedClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn fixed_clock() -> Arc<dyn GatewayRequestClock> {
    Arc::new(FixedClock(
        Utc.with_ymd_and_hms(2026, 3, 15, 23, 30, 0)
            .single()
            .expect("the fixed instant"),
    ))
}

pub(crate) struct FakeStore {
    rows: Vec<GatewayActivityRow>,
    failure: Mutex<Option<DbError>>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeStore {
    pub(crate) fn answering(rows: Vec<GatewayActivityRow>) -> Self {
        Self {
            rows,
            failure: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self::failing_with(DbError::Pool {
            source: PoolFailure {
                driver: PoolError::Timeout(TimeoutType::Wait),
            },
        })
    }

    pub(crate) fn failing_with(error: DbError) -> Self {
        Self {
            rows: Vec::new(),
            failure: Mutex::new(Some(error)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn erased(store: &Arc<Self>) -> Arc<dyn GatewayActivityStore> {
        Arc::<Self>::clone(store)
    }

    pub(crate) fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl GatewayActivityStore for FakeStore {
    async fn aggregate(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<GatewayActivityRow>, DbError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((start_date.to_owned(), end_date.to_owned()));
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(self.rows.clone())
    }
}

// The test database raises the two driver errors LiteLLM classifies: a stale plan and a data fault
pub(crate) async fn cached_plan_error() -> DbError {
    let (fixture, pool) = gateway_database().await;
    let client = pool.get().await.expect("a pooled client");
    let statement = client
        .prepare(r#"SELECT * FROM "LiteLLM_DailyGatewayRequests""#)
        .await
        .expect("the prepared statement");
    client
        .batch_execute(
            r#"ALTER TABLE "LiteLLM_DailyGatewayRequests" ADD COLUMN "activity_probe" text"#,
        )
        .await
        .expect("the added column");
    let error = client
        .query(&statement, &[])
        .await
        .expect_err("the cached plan no longer matches the table");
    drop(client);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
    backend(error)
}

pub(crate) async fn data_error() -> DbError {
    let (fixture, pool) = gateway_database().await;
    let client = pool.get().await.expect("a pooled client");
    let error = client
        .batch_execute("SELECT 1 / 0")
        .await
        .expect_err("the division fails");
    drop(client);
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
    backend(error)
}

pub(crate) fn row(
    date: &str,
    category: &str,
    route: &str,
    successful_requests: i64,
    failed_requests: i64,
) -> GatewayActivityRow {
    GatewayActivityRow {
        date: date.to_owned(),
        category: category.to_owned(),
        route: route.to_owned(),
        successful_requests,
        failed_requests,
    }
}

pub(crate) fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(crate) fn admin() -> AuthContext {
    caller("user", vec![Role::PlatformAdmin])
}

pub(crate) fn app(store: Option<Arc<dyn GatewayActivityStore>>) -> Router {
    router(GatewayActivityState {
        store,
        clock: fixed_clock(),
    })
}

pub(crate) async fn get(app: Router, caller: Option<&AuthContext>, uri: &str) -> Reply {
    let mut request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds");
    if let Some(context) = caller {
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
    pub(crate) body: Vec<u8>,
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
pub(crate) struct ErrorEnvelope {
    pub(crate) error: ErrorFields,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorFields {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) error_type: String,
    pub(crate) param: String,
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalEnvelope {
    pub(crate) error: InternalFields,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InternalFields {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) error_type: String,
}

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn gateway_database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["gateway-activity-tests"])
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

pub(crate) fn postgres_store(pool: &Pool) -> PostgresActivityStore {
    PostgresActivityStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    }
}

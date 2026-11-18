#![cfg(test)]

use std::convert::Infallible;
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
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
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

// A listener that answers the startup with an ErrorResponse yields the real driver error LiteLLM sees
pub(crate) async fn startup_error(code: &str, message: &str) -> DbError {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("the fake postgres listener");
    let address = listener.local_addr().expect("the fake postgres address");
    let frame = error_frame(code, message);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut header = [0_u8; 4];
        if socket.read_exact(&mut header).await.is_err() {
            return;
        }
        let size = u32::from_be_bytes(header).saturating_sub(4);
        let mut startup = vec![0_u8; usize::try_from(size).unwrap_or_default()];
        if socket.read_exact(&mut startup).await.is_err() {
            return;
        }
        let _written = socket.write_all(&frame).await;
    });
    let dsn = format!("host=127.0.0.1 port={} user=gateway", address.port());
    let Err(driver) = tokio_postgres::connect(&dsn, NoTls).await else {
        panic!("the fake postgres server never completes a startup");
    };
    if let Some(result) = tasks.join_next().await {
        result.expect("the fake postgres task");
    }
    let database = driver.as_db_error().expect("the fake postgres error");
    assert_eq!(database.code().code(), code);
    assert_eq!(database.message(), message);
    backend::<Infallible>(driver)
}

fn error_frame(code: &str, message: &str) -> Vec<u8> {
    let mut body: Vec<u8> = [
        (b'S', "FATAL"),
        (b'V', "FATAL"),
        (b'C', code),
        (b'M', message),
    ]
    .into_iter()
    .flat_map(|(field, value)| [field].into_iter().chain(value.bytes()).chain([0_u8]))
    .collect();
    body.push(0);
    let mut frame = vec![b'E'];
    let size = u32::try_from(body.len().saturating_add(4)).expect("the error frame length");
    frame.extend_from_slice(&size.to_be_bytes());
    frame.extend(body);
    frame
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

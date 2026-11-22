#![cfg(test)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::Request;
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::DateTime;
use chrono::NaiveDateTime;
use chrono::TimeDelta;
use chrono::TimeZone as _;
use chrono::Utc;
use clap::Parser;
use deadpool_postgres::Manager;
use deadpool_postgres::Pool;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::model_metrics_routes::ModelMetricsState;
use crate::model_metrics_routes::routes;
use crate::model_metrics_routes::store::ExceptionRow;
use crate::model_metrics_routes::store::ExceptionWindow;
use crate::model_metrics_routes::store::ModelMetricsStore;
use crate::model_metrics_routes::store::PostgresModelMetricsStore;
use crate::model_metrics_routes::store::SeriesValue;
use crate::model_metrics_routes::store::SlowRequest;
use crate::model_metrics_routes::store::SlowRow;
use crate::model_metrics_routes::store::SpendFilters;
use crate::model_metrics_routes::store::StreamingWindow;

const SPEND_TABLES: &str = r#"
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id text PRIMARY KEY,
    call_type text NOT NULL DEFAULT '',
    api_key text NOT NULL DEFAULT '',
    model text NOT NULL DEFAULT '',
    model_group text,
    api_base text,
    "startTime" timestamp(3) NOT NULL,
    "endTime" timestamp(3) NOT NULL,
    "completionStartTime" timestamp(3),
    cache_hit text,
    completion_tokens integer NOT NULL DEFAULT 0,
    end_user text
);
CREATE TABLE "LiteLLM_ErrorLogs" (
    request_id text PRIMARY KEY,
    "startTime" timestamp(3) NOT NULL,
    "endTime" timestamp(3) NOT NULL,
    api_base text NOT NULL DEFAULT '',
    model_group text NOT NULL DEFAULT '',
    litellm_model_name text NOT NULL DEFAULT '',
    exception_type text NOT NULL DEFAULT ''
);
"#;

pub(crate) const DEFAULT_THRESHOLD: f64 = 300.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    SameDay(StreamingWindow),
    Daily(StreamingWindow),
    Latency(SpendFilters),
    Slow(SlowRequest),
    Exceptions(ExceptionWindow),
}

#[derive(Default, Clone)]
pub(crate) struct Rows {
    pub(crate) same_day: Vec<SeriesValue>,
    pub(crate) daily: Vec<SeriesValue>,
    pub(crate) latency: Vec<SeriesValue>,
    pub(crate) slow: Vec<SlowRow>,
    pub(crate) exceptions: Vec<ExceptionRow>,
}

#[derive(Default)]
pub(crate) struct FakeStore {
    pub(crate) rows: Rows,
    pub(crate) failure: bool,
    calls: Mutex<Vec<Call>>,
}

impl FakeStore {
    fn record(&self, call: Call) -> Result<(), DbError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        if self.failure {
            return Err(DbError::NoDbConnected);
        }
        Ok(())
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ModelMetricsStore for FakeStore {
    async fn streaming_same_day(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SeriesValue>, DbError> {
        self.record(Call::SameDay(request.clone()))?;
        Ok(self.rows.same_day.clone())
    }

    async fn streaming_daily(
        &self,
        request: &StreamingWindow,
    ) -> Result<Vec<SeriesValue>, DbError> {
        self.record(Call::Daily(request.clone()))?;
        Ok(self.rows.daily.clone())
    }

    async fn latency(&self, request: &SpendFilters) -> Result<Vec<SeriesValue>, DbError> {
        self.record(Call::Latency(request.clone()))?;
        Ok(self.rows.latency.clone())
    }

    async fn slow_responses(&self, request: &SlowRequest) -> Result<Vec<SlowRow>, DbError> {
        self.record(Call::Slow(request.clone()))?;
        Ok(self.rows.slow.clone())
    }

    async fn exceptions(&self, request: &ExceptionWindow) -> Result<Vec<ExceptionRow>, DbError> {
        self.record(Call::Exceptions(request.clone()))?;
        Ok(self.rows.exceptions.clone())
    }
}

struct FixedClock(DateTime<Utc>);

impl GatewayRequestClock for FixedClock {
    fn utc_now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 23, 30, 0)
        .single()
        .expect("the fixed instant")
}

pub(crate) fn clock() -> Arc<dyn GatewayRequestClock> {
    Arc::new(FixedClock(fixed_now()))
}

pub(crate) fn app(store: Option<Arc<dyn ModelMetricsStore>>, threshold: f64) -> Router {
    routes(ModelMetricsState {
        store,
        clock: clock(),
        alerting_threshold_seconds: threshold,
    })
}

pub(crate) fn caller() -> AuthContext {
    let principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user","roles":[],"auth_method":"api_key"}"#,
    )
    .expect("the test principal");
    AuthContext { principal }
}

#[derive(Debug)]
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: String,
}

pub(crate) async fn get(router: Router, caller: Option<&AuthContext>, uri: &str) -> Reply {
    let mut request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds");
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    let response = router.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body");
    Reply {
        status,
        body: String::from_utf8(body.to_vec()).expect("the response is UTF-8"),
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["model-metrics-routes-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![SPEND_TABLES.to_owned()])
            .resolve()
            .await
            .expect("the postgres fixture");
    let settings = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let manager = Manager::new(settings, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the postgres pool");
    (fixture, pool)
}

pub(crate) async fn execute(pool: &Pool, sql: &str) {
    let client = pool.get().await.expect("the pooled client");
    client.batch_execute(sql).await.expect("the seed SQL");
}

pub(crate) fn postgres_store(pool: &Pool) -> PostgresModelMetricsStore {
    PostgresModelMetricsStore {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
    }
}

pub(crate) fn instant(text: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
        .expect("the calendar instant")
        .and_utc()
}

pub(crate) fn days_before(now: DateTime<Utc>, days: i64) -> DateTime<Utc> {
    now.checked_sub_signed(TimeDelta::days(days))
        .expect("the window start")
}

pub(crate) fn named_latency_series() -> Vec<SeriesValue> {
    vec![
        SeriesValue {
            key: "2026-03-11".to_owned(),
            api_base: Some("http://plain.example.com".to_owned()),
            model: "claude".to_owned(),
            value: 0.008,
        },
        SeriesValue {
            key: "2026-03-10".to_owned(),
            api_base: Some("https://api.example.com/openai/v1".to_owned()),
            model: "gpt-4".to_owned(),
            value: 0.002,
        },
        SeriesValue {
            key: "2026-03-10".to_owned(),
            api_base: None,
            model: "claude".to_owned(),
            value: 0.004,
        },
    ]
}

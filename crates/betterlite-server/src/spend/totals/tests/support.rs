use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_logging::prometheus_query::daily_spend::PrometheusDailySpend;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::spend::totals::prometheus::DailySpendSource;
use crate::spend::totals::rows::{
    DailySpend, KeyDailySpend, KeySpend, ModelSpend, TeamSpend, TotalSpendRow, UserKeyDailySpend,
    UserModelSpend,
};
use crate::spend::totals::state::TotalsState;
use crate::spend::totals::store::{TotalsStore, TotalsStoreError};

type Fetched<Row> = Result<Vec<Row>, TotalsStoreError>;

#[derive(Clone, Copy)]
pub(super) enum FakeFailure {
    NoDatabase,
    InvalidAmount,
}

pub(super) struct FakeStore {
    pub(super) global_daily: Vec<DailySpend>,
    pub(super) key_daily: Vec<KeyDailySpend>,
    pub(super) user_daily: Vec<UserKeyDailySpend>,
    pub(super) total: Vec<TotalSpendRow>,
    pub(super) keys: Vec<KeySpend>,
    pub(super) user_keys: Vec<KeySpend>,
    pub(super) models: Vec<ModelSpend>,
    pub(super) user_models: Vec<UserModelSpend>,
    pub(super) teams: Vec<TeamSpend>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) failure: Option<FakeFailure>,
}

pub(super) fn spend_store() -> FakeStore {
    FakeStore {
        global_daily: Vec::new(),
        key_daily: Vec::new(),
        user_daily: Vec::new(),
        total: Vec::new(),
        keys: Vec::new(),
        user_keys: Vec::new(),
        models: Vec::new(),
        user_models: Vec::new(),
        teams: Vec::new(),
        calls: Arc::new(Mutex::new(Vec::new())),
        failure: None,
    }
}

pub(super) fn prometheus() -> FakePrometheus {
    FakePrometheus {
        rows: Vec::new(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fails: false,
    }
}

impl FakeStore {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn answer<Row>(&self, call: String, rows: &[Row]) -> Fetched<Row>
    where
        Row: Clone,
    {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.failure {
            Some(FakeFailure::NoDatabase) => {
                Err(TotalsStoreError::Database(DbError::NoDbConnected))
            }
            Some(FakeFailure::InvalidAmount) => {
                Err(TotalsStoreError::InvalidAmount { column: "spend" })
            }
            None => Ok(rows.to_vec()),
        }
    }
}

#[async_trait]
impl TotalsStore for FakeStore {
    async fn global_daily_spend(&self) -> Fetched<DailySpend> {
        self.answer("global_daily_spend".to_owned(), &self.global_daily)
    }

    async fn key_daily_spend(&self, api_key: &str) -> Fetched<KeyDailySpend> {
        self.answer(format!("key_daily_spend({api_key})"), &self.key_daily)
    }

    async fn user_daily_spend(
        &self,
        user_id: &str,
        api_key: Option<&str>,
    ) -> Fetched<UserKeyDailySpend> {
        self.answer(
            format!("user_daily_spend({user_id},{api_key:?})"),
            &self.user_daily,
        )
    }

    async fn global_total_spend(&self) -> Fetched<TotalSpendRow> {
        self.answer("global_total_spend".to_owned(), &self.total)
    }

    async fn keys_by_spend(&self, limit: Option<i64>) -> Fetched<KeySpend> {
        self.answer(format!("keys_by_spend({limit:?})"), &self.keys)
    }

    async fn user_keys_by_spend(&self, user_id: &str, limit: i64) -> Fetched<KeySpend> {
        self.answer(
            format!("user_keys_by_spend({user_id},{limit})"),
            &self.user_keys,
        )
    }

    async fn models_by_spend(&self, limit: i64) -> Fetched<ModelSpend> {
        self.answer(format!("models_by_spend({limit})"), &self.models)
    }

    async fn user_models_by_spend(&self, user_id: &str, limit: i64) -> Fetched<UserModelSpend> {
        self.answer(
            format!("user_models_by_spend({user_id},{limit})"),
            &self.user_models,
        )
    }

    async fn team_daily_spend(&self) -> Fetched<TeamSpend> {
        self.answer("team_daily_spend".to_owned(), &self.teams)
    }
}

pub(super) struct FakePrometheus {
    pub(super) rows: Vec<PrometheusDailySpend>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) fails: bool,
}

impl FakePrometheus {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake Prometheus source failed")]
pub(super) struct FakePrometheusFailure;

#[async_trait]
impl DailySpendSource for FakePrometheus {
    type Failure = FakePrometheusFailure;

    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, Self::Failure> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("daily_spend({})", api_key.unwrap_or("<all>")));
        if self.fails {
            return Err(FakePrometheusFailure);
        }
        Ok(self.rows.clone())
    }
}

pub(super) fn state(
    store: Option<&Arc<FakeStore>>,
    prometheus: Option<&Arc<FakePrometheus>>,
    max_budget: Decimal,
) -> TotalsState<FakePrometheus> {
    let shared: Option<Arc<dyn TotalsStore>> = store.map(|fake| {
        let cloned: Arc<dyn TotalsStore> = Arc::<FakeStore>::clone(fake);
        cloned
    });
    TotalsState {
        store: shared,
        prometheus: prometheus.map(Arc::clone),
        max_budget,
    }
}

fn principal(subject: &str, roles: Vec<Role>, user_id: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(super) fn admin() -> Principal {
    principal("virtual-key", vec![Role::PlatformAdmin], None)
}

pub(super) fn internal(user_id: &str) -> Principal {
    principal("virtual-key", Vec::new(), Some(user_id))
}

pub(super) fn internal_without_user() -> Principal {
    principal("virtual-key", Vec::new(), None)
}

pub(super) fn role_holder(role: Role, user_id: &str) -> Principal {
    principal("virtual-key", vec![role], Some(user_id))
}

pub(super) fn master_key() -> Principal {
    master_caller().principal
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

pub(super) async fn call(
    uri: &str,
    identity: Option<&Principal>,
    state: TotalsState<FakePrometheus>,
) -> Reply {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    if let Some(holder) = identity {
        request.extensions_mut().insert(AuthContext {
            principal: holder.clone(),
        });
    }
    reply(super::super::routes(state), request).await
}

async fn reply(application: Router, request: Request<Body>) -> Reply {
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply { status, body }
}

pub(super) fn decode<T>(reply: &Reply) -> T
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(&reply.body).unwrap()
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(super) fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

pub(super) fn body_text(reply: &Reply) -> String {
    String::from_utf8(reply.body.clone()).unwrap()
}

pub(super) fn retained(value: f64) -> Decimal {
    Decimal::from_f64_retain(value).unwrap()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DailyBody {
    pub(super) date: String,
    pub(super) spend: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeyDailyBody {
    pub(super) date: String,
    pub(super) spend: Decimal,
    pub(super) api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UserKeyDailyBody {
    pub(super) date: String,
    pub(super) spend: Decimal,
    pub(super) api_key: String,
    pub(super) user: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TotalBody {
    pub(super) spend: Option<Decimal>,
    pub(super) max_budget: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KeyBody {
    pub(super) api_key: String,
    pub(super) total_spend: Decimal,
    pub(super) key_alias: Option<String>,
    pub(super) key_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ModelBody {
    pub(super) model: String,
    pub(super) total_spend: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UserModelBody {
    pub(super) model: String,
    pub(super) total_spend: Decimal,
    pub(super) total_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamsBody {
    pub(super) daily_spend: Vec<TeamDayBody>,
    pub(super) teams: Vec<String>,
    pub(super) total_spend_per_team: Vec<TeamTotalBody>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TeamDayBody {
    pub(super) date: String,
    #[serde(flatten)]
    pub(super) amounts: BTreeMap<String, Decimal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamTotalBody {
    pub(super) team_id: String,
    pub(super) total_spend: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorEnvelope {
    pub(super) error: ErrorBody,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ErrorBody {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) param: String,
    pub(super) code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InternalEnvelope {
    pub(super) error: InternalBody,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InternalBody {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetailMessageEnvelope {
    pub(super) detail: DetailMessage,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetailMessage {
    pub(super) error: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ValidationEnvelope {
    pub(super) detail: Vec<ValidationIssue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationIssue {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) loc: Vec<String>,
    pub(super) msg: String,
}

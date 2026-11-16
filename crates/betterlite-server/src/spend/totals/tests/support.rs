use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

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
use crate::spend::totals::prometheus::{DailySpendFailure, DailySpendSource};
use crate::spend::totals::rows::{
    DailySpend, KeyDailySpend, KeySpend, ModelSpend, TeamSpend, TotalSpendRow, UserKeyDailySpend,
    UserModelSpend,
};
use crate::spend::totals::state::TotalsState;
use crate::spend::totals::store::{TotalsStore, TotalsStoreError};

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

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn answer<T: Clone>(&self, rows: &[T]) -> Result<Vec<T>, TotalsStoreError> {
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
    async fn global_daily_spend(&self) -> Result<Vec<DailySpend>, TotalsStoreError> {
        self.record("global_daily_spend".to_owned());
        self.answer(&self.global_daily)
    }

    async fn key_daily_spend(&self, api_key: &str) -> Result<Vec<KeyDailySpend>, TotalsStoreError> {
        self.record(format!("key_daily_spend({api_key})"));
        self.answer(&self.key_daily)
    }

    async fn user_daily_spend(
        &self,
        user_id: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<UserKeyDailySpend>, TotalsStoreError> {
        self.record(format!("user_daily_spend({user_id},{api_key:?})"));
        self.answer(&self.user_daily)
    }

    async fn global_total_spend(&self) -> Result<Vec<TotalSpendRow>, TotalsStoreError> {
        self.record("global_total_spend".to_owned());
        self.answer(&self.total)
    }

    async fn keys_by_spend(&self, limit: Option<i64>) -> Result<Vec<KeySpend>, TotalsStoreError> {
        self.record(format!("keys_by_spend({limit:?})"));
        self.answer(&self.keys)
    }

    async fn user_keys_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<KeySpend>, TotalsStoreError> {
        self.record(format!("user_keys_by_spend({user_id},{limit})"));
        self.answer(&self.user_keys)
    }

    async fn models_by_spend(&self, limit: i64) -> Result<Vec<ModelSpend>, TotalsStoreError> {
        self.record(format!("models_by_spend({limit})"));
        self.answer(&self.models)
    }

    async fn user_models_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<UserModelSpend>, TotalsStoreError> {
        self.record(format!("user_models_by_spend({user_id},{limit})"));
        self.answer(&self.user_models)
    }

    async fn team_daily_spend(&self) -> Result<Vec<TeamSpend>, TotalsStoreError> {
        self.record("team_daily_spend".to_owned());
        self.answer(&self.teams)
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

#[async_trait]
impl DailySpendSource for FakePrometheus {
    async fn daily_spend(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<PrometheusDailySpend>, DailySpendFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("daily_spend({})", api_key.unwrap_or("<all>")));
        if self.fails {
            return Err(DailySpendFailure::Query);
        }
        Ok(self.rows.clone())
    }
}

pub(super) fn state(
    store: Option<&Arc<FakeStore>>,
    prometheus: Option<&Arc<FakePrometheus>>,
    max_budget: Decimal,
) -> TotalsState {
    let store: Option<Arc<dyn TotalsStore>> = store.map(|store| {
        let cloned: Arc<dyn TotalsStore> = Arc::<FakeStore>::clone(store);
        cloned
    });
    let prometheus: Option<Arc<dyn DailySpendSource>> = prometheus.map(|source| {
        let cloned: Arc<dyn DailySpendSource> = Arc::<FakePrometheus>::clone(source);
        cloned
    });
    TotalsState {
        store,
        prometheus,
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

pub(super) fn master_key() -> Principal {
    principal("master_key", Vec::new(), None)
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

pub(super) async fn call(uri: &str, identity: Option<&Principal>, state: TotalsState) -> Reply {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    if let Some(identity) = identity {
        request.extensions_mut().insert(AuthContext {
            principal: identity.clone(),
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

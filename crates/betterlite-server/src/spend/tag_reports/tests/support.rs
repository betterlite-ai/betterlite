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
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::{RowDecoder as _, RowError};
use rust_decimal::Decimal;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::spend::tag_reports::rows::{DailyTagSpend, KeyAliasedUser, RawTagSpend};
use crate::spend::tag_reports::store::{TagReportError, TagReportHandle, TagReportStore};

#[derive(Clone, Copy, Debug)]
pub(super) enum FakeFailure {
    NoDatabase,
    Undecodable,
}

pub(super) struct FakeStore {
    pub(super) keys: Vec<LiteLLMVerificationToken>,
    pub(super) user_keys: Vec<LiteLLMVerificationToken>,
    pub(super) users: Vec<KeyAliasedUser>,
    pub(super) user: Option<LiteLLMUserTable>,
    pub(super) raw_tags: Vec<RawTagSpend>,
    pub(super) names: Option<Vec<Option<String>>>,
    pub(super) daily: Vec<DailyTagSpend>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) failure: Option<FakeFailure>,
}

pub(super) fn tag_store() -> FakeStore {
    FakeStore {
        keys: Vec::new(),
        user_keys: Vec::new(),
        users: Vec::new(),
        user: None,
        raw_tags: Vec::new(),
        names: Some(Vec::new()),
        daily: Vec::new(),
        calls: Arc::new(Mutex::new(Vec::new())),
        failure: None,
    }
}

impl FakeStore {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) -> Result<(), TagReportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.failure {
            Some(FakeFailure::NoDatabase) => Err(TagReportError::Database(DbError::NoDbConnected)),
            Some(FakeFailure::Undecodable) => Err(TagReportError::Row(RowError::NotAnObject)),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl TagReportStore for FakeStore {
    async fn keys(&self) -> Result<Vec<LiteLLMVerificationToken>, TagReportError> {
        self.record("keys".to_owned())?;
        Ok(self.keys.clone())
    }

    async fn keys_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TagReportError> {
        self.record(format!("keys_for_user({user_id})"))?;
        Ok(self.user_keys.clone())
    }

    async fn users(&self) -> Result<Vec<KeyAliasedUser>, TagReportError> {
        self.record("users".to_owned())?;
        Ok(self.users.clone())
    }

    async fn user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, TagReportError> {
        self.record(format!("user({user_id})"))?;
        Ok(self.user.clone())
    }

    async fn raw_tag_spend(&self) -> Result<Vec<RawTagSpend>, TagReportError> {
        self.record("raw_tag_spend".to_owned())?;
        Ok(self.raw_tags.clone())
    }

    async fn tag_names(&self) -> Result<Option<Vec<Option<String>>>, TagReportError> {
        self.record("tag_names".to_owned())?;
        Ok(self.names.clone())
    }

    async fn daily_tag_spend(
        &self,
        start_date: &str,
        end_date: &str,
        tags: Option<&[String]>,
    ) -> Result<Vec<DailyTagSpend>, TagReportError> {
        self.record(format!("daily_tag_spend({start_date},{end_date},{tags:?})"))?;
        Ok(self.daily.clone())
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

pub(super) fn viewer() -> Principal {
    principal("virtual-key", vec![Role::PlatformViewer], None)
}

pub(super) fn internal(user_id: &str) -> Principal {
    principal("virtual-key", Vec::new(), Some(user_id))
}

pub(super) fn internal_without_user() -> Principal {
    principal("virtual-key", Vec::new(), None)
}

pub(super) fn org_admin(user_id: &str) -> Principal {
    principal("virtual-key", vec![Role::OrgAdmin], Some(user_id))
}

pub(super) fn master_key() -> Principal {
    master_caller().principal
}

pub(super) fn key_principal(fields: &str) -> Principal {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap();
    let mut holder = principal("virtual-key", Vec::new(), None);
    holder.source_key = SourceKey(Some(key));
    holder
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub(super) fn decode<T>(&self) -> T
    where
        T: for<'de> Deserialize<'de>,
    {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(super) async fn call(
    uri: &str,
    identity: Option<&Principal>,
    store: Option<&Arc<FakeStore>>,
) -> Reply {
    let report: TagReportHandle = store.map(|fake| {
        let handle: Arc<dyn TagReportStore> = Arc::<FakeStore>::clone(fake);
        handle
    });
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
    reply(super::super::routes(report), request).await
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

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(super) fn key(record: &str) -> LiteLLMVerificationToken {
    LiteLLMVerificationToken::decode_row(record).unwrap()
}

pub(super) fn user(record: &str) -> LiteLLMUserTable {
    LiteLLMUserTable::decode_row(record).unwrap()
}

pub(super) fn aliased(user: LiteLLMUserTable, aliases: &[Option<&str>]) -> KeyAliasedUser {
    KeyAliasedUser {
        user,
        key_aliases: aliases
            .iter()
            .map(|alias| alias.map(str::to_owned))
            .collect(),
    }
}

pub(super) fn raw_tag(name: Option<&str>, log_count: i64, total_spend: &str) -> RawTagSpend {
    RawTagSpend {
        individual_request_tag: name.map(str::to_owned),
        log_count,
        total_spend: decimal(total_spend),
    }
}

pub(super) fn daily_tag(name: Option<&str>, total_spend: &str, log_count: i64) -> DailyTagSpend {
    DailyTagSpend {
        individual_request_tag: name.map(str::to_owned),
        total_spend: decimal(total_spend),
        log_count,
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct KeyBody {
    pub(super) token: Option<String>,
    pub(super) key_alias: Option<String>,
    pub(super) key_name: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) litellm_budget_table: Option<BudgetBody>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BudgetBody {
    pub(super) budget_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UserBody {
    pub(super) user_id: String,
    pub(super) spend: Decimal,
    #[serde(default)]
    pub(super) key_aliases: Option<Vec<Option<String>>>,
    #[serde(default)]
    pub(super) organization_memberships: Option<Vec<MembershipBody>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MembershipBody {
    pub(super) organization_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawTagBody {
    pub(super) individual_request_tag: Option<String>,
    pub(super) log_count: i64,
    pub(super) total_spend: Decimal,
}

#[derive(Debug, Deserialize)]
pub(super) struct TagNamesBody {
    pub(super) tag_names: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpendPerTagBody {
    pub(super) spend_per_tag: Vec<SpendPerTagRow>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpendPerTagRow {
    pub(super) name: Option<String>,
    pub(super) spend: Decimal,
    pub(super) log_count: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorEnvelope {
    pub(super) error: ErrorObjectBody,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorObjectBody {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) param: Option<String>,
    pub(super) code: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct DetailEnvelope {
    pub(super) detail: DetailErrorBody,
}

#[derive(Debug, Deserialize)]
pub(super) struct DetailErrorBody {
    pub(super) error: String,
}

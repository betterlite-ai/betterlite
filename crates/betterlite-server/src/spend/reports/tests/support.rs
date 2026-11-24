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
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowDecoder as _;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::organization_create::access::OrgAccessError;
use crate::spend::reports::rows::{
    DailyTeamSpend, ModelSpend, OrganizationKeySpend, OrganizationModelSpend, ScopedKeySpend,
};
use crate::spend::reports::store::{
    OrganizationReportRequest, ScopedReportError, ScopedReportHandle, ScopedReportRequest,
    ScopedReportStore, TeamSpendRequest,
};

pub(super) struct FakeStore {
    pub(super) scoped: Vec<ScopedKeySpend>,
    pub(super) organization: Vec<OrganizationKeySpend>,
    pub(super) team_ids: Vec<String>,
    pub(super) users: BTreeMap<String, LiteLLMUserTable>,
    pub(super) daily: Vec<DailyTeamSpend>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) failure: bool,
}

pub(super) fn fake_store() -> FakeStore {
    FakeStore {
        scoped: Vec::new(),
        organization: Vec::new(),
        team_ids: Vec::new(),
        users: BTreeMap::new(),
        daily: Vec::new(),
        calls: Arc::new(Mutex::new(Vec::new())),
        failure: false,
    }
}

impl FakeStore {
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) -> Result<(), ScopedReportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        if self.failure {
            return Err(ScopedReportError::Database(DbError::NoDbConnected));
        }
        Ok(())
    }
}

fn report_range(start: &DateTime<Utc>, end: &DateTime<Utc>) -> String {
    format!("{},{}", start.to_rfc3339(), end.to_rfc3339())
}

fn scoped_call(request: ScopedReportRequest<'_>) -> String {
    let range = report_range(request.start, request.end);
    let column = request.column;
    format!("scoped_report({column:?},{range},{})", request.value)
}

fn organization_call(request: OrganizationReportRequest<'_>) -> String {
    let listed = format!("{:?}", request.team_ids);
    let range = report_range(request.start, request.end);
    let id = request.organization_id;
    format!("organization_report({range},{id},{listed})")
}

#[async_trait]
impl ScopedReportStore for FakeStore {
    async fn scoped_report(
        &self,
        request: ScopedReportRequest<'_>,
    ) -> Result<Vec<ScopedKeySpend>, ScopedReportError> {
        self.record(scoped_call(request))?;
        Ok(self.scoped.clone())
    }

    async fn organization_report(
        &self,
        request: OrganizationReportRequest<'_>,
    ) -> Result<Vec<OrganizationKeySpend>, ScopedReportError> {
        self.record(organization_call(request))?;
        Ok(self.organization.clone())
    }

    async fn organization_team_ids(
        &self,
        organization_id: &str,
    ) -> Result<Vec<String>, ScopedReportError> {
        self.record(format!("organization_team_ids({organization_id})"))?;
        Ok(self.team_ids.clone())
    }

    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        Ok(self.users.get(user_id).cloned())
    }

    async fn spend_by_team(
        &self,
        _request: TeamSpendRequest<'_>,
    ) -> Result<Vec<DailyTeamSpend>, ScopedReportError> {
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

pub(super) fn master_key() -> Principal {
    principal("master_key", Vec::new(), None)
}

pub(super) fn jwt_user(user_id: &str) -> Principal {
    principal("jwt", Vec::new(), Some(user_id))
}

pub(super) fn key_principal(fields: &str) -> Principal {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap();
    let mut holder = principal("virtual-key", Vec::new(), None);
    holder.source_key = SourceKey(Some(key));
    holder
}

pub(super) fn user(record: &str) -> LiteLLMUserTable {
    LiteLLMUserTable::decode_row(record).unwrap()
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
    let report: ScopedReportHandle = store.map(|fake| {
        let handle: Arc<dyn ScopedReportStore> = Arc::<FakeStore>::clone(fake);
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

pub(super) fn model(model: &str, total: &str, input: i64, output: i64) -> ModelSpend {
    ModelSpend {
        model: model.to_owned(),
        total_cost: decimal(total),
        total_input_tokens: input,
        total_output_tokens: output,
    }
}

pub(super) fn scoped_row(api_key: &str, total: &str, models: Vec<ModelSpend>) -> ScopedKeySpend {
    ScopedKeySpend {
        api_key: api_key.to_owned(),
        total_cost: decimal(total),
        total_input_tokens: models.iter().map(|detail| detail.total_input_tokens).sum(),
        total_output_tokens: models.iter().map(|detail| detail.total_output_tokens).sum(),
        model_details: models,
    }
}

pub(super) fn organization_model(
    team_id: Option<&str>,
    name: &str,
    total: &str,
) -> OrganizationModelSpend {
    OrganizationModelSpend {
        team_id: team_id.map(str::to_owned),
        model: name.to_owned(),
        total_cost: decimal(total),
        total_input_tokens: 1,
        total_output_tokens: 2,
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DetailBody {
    pub(super) detail: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InternalBody {
    pub(super) error: InternalMessage,
}

#[derive(Debug, Deserialize)]
pub(super) struct InternalMessage {
    pub(super) message: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
}

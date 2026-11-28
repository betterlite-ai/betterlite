use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::health_access_and_connections::connection_state::{
    ConnectionProbe, ConnectionProbeError, ModelCallAuthorizer, TestConnectionState,
};
use crate::health_access_and_connections::connection_test::router;
use crate::health_access_and_connections::visibility::{HealthRouter, LookupFailure};
use crate::model_write_fields::teams::TeamRows;
use crate::team_access::access::{OrgRoles, TeamAccess};
use crate::team_access::error::TeamAccessError;

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn deployment(
    model_name: &str,
    id: &str,
    params: Vec<(&str, Json)>,
    info: Vec<(&str, Json)>,
) -> DeploymentSpec {
    let mut model_info = object(info);
    model_info.insert("id".to_owned(), text(id));
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: object(params),
        model_info,
        extra: JsonObject::new(),
    }
}

pub(super) fn team_deployment(model_name: &str, id: &str, team_id: &str) -> DeploymentSpec {
    deployment(model_name, id, Vec::new(), vec![("team_id", text(team_id))])
}

pub(super) struct FakeRouter {
    pub(super) deployments: Vec<DeploymentSpec>,
    pub(super) access_groups: AccessGroupMap,
    pub(super) fail: bool,
}

pub(super) fn fake_router(deployments: Vec<DeploymentSpec>) -> FakeRouter {
    FakeRouter {
        deployments,
        access_groups: AccessGroupMap::new(),
        fail: false,
    }
}

pub(super) fn failing_router() -> FakeRouter {
    FakeRouter {
        deployments: Vec::new(),
        access_groups: AccessGroupMap::new(),
        fail: true,
    }
}

// Router.should_include_deployment's team arm: no team constraint includes every row, a team includes its own and the global ones
fn team_allows(row: &DeploymentSpec, team_id: Option<&str>) -> bool {
    let Some(team) = team_id else {
        return true;
    };
    match row.model_info.get("team_id") {
        Some(Json::Text(owner)) => owner.as_str() == team,
        _ => true,
    }
}

impl FakeRouter {
    fn guard(&self) -> Result<(), LookupFailure> {
        if self.fail {
            return Err(LookupFailure::Lookup(LookupError::InvalidRow { index: 0 }));
        }
        Ok(())
    }

    fn scoped(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.guard()?;
        Ok(self
            .deployments
            .iter()
            .filter(|row| model_name.is_none_or(|name| row.model_name == name))
            .filter(|row| team_allows(row, team_id))
            .cloned()
            .collect())
    }
}

impl HealthRouter for FakeRouter {
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        self.guard()?;
        let found = self
            .deployments
            .iter()
            .find(|row| row.id() == Some(model_id));
        Ok(found.cloned())
    }

    fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.scoped(model_name, team_id)
    }

    fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        let rows = self.scoped(None, team_id)?;
        Ok(rows.into_iter().map(|row| row.model_name).collect())
    }

    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        self.guard()?;
        Ok(self.access_groups.clone())
    }
}

pub(super) struct FakeProbe {
    pub(super) result: JsonObject,
    pub(super) failure: Option<String>,
    pub(super) seen: Mutex<Vec<(JsonObject, Option<String>)>>,
}

impl FakeProbe {
    pub(super) fn answering(result: JsonObject) -> Arc<Self> {
        Arc::new(Self {
            result,
            failure: None,
            seen: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            result: JsonObject::new(),
            failure: Some(message.to_owned()),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn seen(&self) -> Vec<(JsonObject, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl ConnectionProbe for FakeProbe {
    async fn probe(
        &self,
        params: &JsonObject,
        mode: Option<&str>,
    ) -> Result<JsonObject, ConnectionProbeError> {
        self.seen
            .lock()
            .unwrap()
            .push((params.clone(), mode.map(str::to_owned)));
        self.failure.as_ref().map_or_else(
            || Ok(self.result.clone()),
            |message| {
                Err(ConnectionProbeError {
                    message: message.clone(),
                })
            },
        )
    }
}

pub(super) fn team(team_id: &str) -> LiteLLMTeamTable {
    let row = format!(
        r#"{{"team_id": "{team_id}", "organization_id": "org-1", "members_with_roles": [{{"user_id": "admin-1", "role": "admin"}}, {{"user_id": "member-1", "role": "user"}}]}}"#
    );
    serde_json::from_str(&row).unwrap()
}

#[derive(Default)]
pub(super) struct FakeTeams {
    pub(super) teams: Vec<LiteLLMTeamTable>,
}

impl FakeTeams {
    pub(super) fn holding(team_ids: &[&str]) -> Self {
        Self {
            teams: team_ids.iter().map(|team_id| team(team_id)).collect(),
        }
    }
}

#[async_trait]
impl TeamRows for FakeTeams {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, TeamQueryError> {
        Ok(self
            .teams
            .iter()
            .find(|stored| stored.fields.base.team_id == team_id)
            .cloned())
    }
}

pub(super) struct NoOrgRoles;

#[async_trait]
impl OrgRoles for NoOrgRoles {
    async fn is_org_admin(
        &self,
        _user_id: &str,
        _organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        Ok(false)
    }
}

fn authorizer(teams: FakeTeams) -> Arc<ModelCallAuthorizer> {
    Arc::new(ModelCallAuthorizer {
        teams: Arc::new(teams),
        team_access: Arc::new(TeamAccess {
            org_roles: Arc::new(NoOrgRoles),
        }),
    })
}

pub(super) fn state_with_router(
    router: Option<Arc<dyn HealthRouter>>,
    fake_probe: &Arc<FakeProbe>,
    teams: FakeTeams,
) -> TestConnectionState {
    let mut bundled = BundledCatalog::default();
    let models = bundled.get();
    let probe: Arc<dyn ConnectionProbe> = Arc::<FakeProbe>::clone(fake_probe);
    TestConnectionState {
        router,
        probe,
        models,
        policy: ProbePolicy::default(),
        allow_client_side_credentials: false,
        authorizer: Some(authorizer(teams)),
    }
}

pub(super) fn state(
    deployments: Vec<DeploymentSpec>,
    probe: &Arc<FakeProbe>,
    teams: FakeTeams,
) -> TestConnectionState {
    let router: Arc<dyn HealthRouter> = Arc::new(fake_router(deployments));
    state_with_router(Some(router), probe, teams)
}

pub(super) fn state_without_database(probe: &Arc<FakeProbe>) -> TestConnectionState {
    let mut connection = state(Vec::new(), probe, FakeTeams::default());
    connection.authorizer = None;
    connection
}

pub(super) fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
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
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(super) fn master_context() -> AuthContext {
    master_caller()
}

pub(super) fn key_context(key_fields: &str) -> AuthContext {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(key_fields).expect("the key fixture parses");
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture builds");
    context("key-owner", &[], SourceKey(Some(key)))
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(&self) -> JsonObject {
        serde_json::from_slice(&self.body).expect("the body is a JSON object")
    }
}

pub(super) async fn call(
    state: TestConnectionState,
    caller: Option<&AuthContext>,
    body: &str,
) -> Reply {
    call_body(state, caller, Body::from(body.to_owned())).await
}

pub(super) async fn call_body(
    state: TestConnectionState,
    caller: Option<&AuthContext>,
    body: Body,
) -> Reply {
    let application = router(state);
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/health/test_connection")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("the request builds");
    if let Some(identity) = caller {
        request.extensions_mut().insert(identity.clone());
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    Reply {
        status,
        body: bytes.to_vec(),
    }
}

pub(super) fn detail_error(body: &JsonObject) -> String {
    let detail = body.get("detail").expect("the body carries a detail");
    match detail {
        Json::Object(fields) => match fields.get("error") {
            Some(Json::Text(message)) => message.clone(),
            _ => panic!("the detail carries an error"),
        },
        Json::Text(message) => message.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            panic!("the detail is text or an object")
        }
    }
}

pub(super) fn single_seen(probe: &FakeProbe) -> (JsonObject, Option<String>) {
    let mut seen = probe.seen();
    assert_eq!(seen.len(), 1, "the probe ran once");
    seen.remove(0)
}

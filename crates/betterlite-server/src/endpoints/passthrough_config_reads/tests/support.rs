#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

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
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::decoder::TeamRecordError;
use betterlite_db::team_repository::repository::TeamQueryError;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::endpoints::passthrough_config_reads::caller::CallerAccess;
use crate::endpoints::passthrough_config_reads::ports::ConfigEndpointsSource;
use crate::endpoints::passthrough_config_reads::ports::GeneralSettingsReader;
use crate::endpoints::passthrough_config_reads::ports::SettingsReadError;
use crate::endpoints::passthrough_config_reads::ports::TeamLookup;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::endpoints::passthrough_config_reads::route::PassThroughConfigState;
use crate::endpoints::passthrough_config_reads::route::routes;

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("test json")
}

pub(crate) fn json_list(entries: &[&str]) -> JsonData {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        values.push(json(entry));
    }
    JsonData::Array(values)
}

pub(crate) fn endpoint(path: &str) -> PassThroughGenericEndpoint {
    let record = json(&format!(
        r#"{{"path": "{path}", "target": "https://target.example.com"}}"#
    ));
    PassThroughGenericEndpoint::from_json(&record, false).expect("an endpoint record")
}

pub(crate) struct FakeConfig {
    pub(crate) entries: Option<Vec<JsonData>>,
}

impl FakeConfig {
    pub(crate) fn none() -> Self {
        Self { entries: None }
    }

    pub(crate) fn of(entries: &[&str]) -> Self {
        let mut values = Vec::with_capacity(entries.len());
        for entry in entries {
            values.push(json(entry));
        }
        Self {
            entries: Some(values),
        }
    }
}

impl ConfigEndpointsSource for FakeConfig {
    fn configured_endpoints(&self) -> Option<&[JsonData]> {
        self.entries.as_deref()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsReply {
    Value,
    Missing,
    NotAllowed,
    Unavailable,
}

pub(crate) struct FakeSettings {
    pub(crate) value: JsonData,
    pub(crate) reply: SettingsReply,
    pub(crate) calls: Mutex<Vec<(String, CallerAccess)>>,
}

impl FakeSettings {
    pub(crate) fn returning(value: JsonData) -> Self {
        Self {
            value,
            reply: SettingsReply::Value,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn refusing(reply: SettingsReply) -> Self {
        Self {
            value: JsonData::Null,
            reply,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn called(&self) -> Vec<(String, CallerAccess)> {
        self.calls.lock().expect("settings call lock").clone()
    }
}

#[async_trait]
impl GeneralSettingsReader for FakeSettings {
    async fn general_setting(
        &self,
        name: &str,
        access: CallerAccess,
    ) -> Result<Option<JsonData>, SettingsReadError> {
        self.calls
            .lock()
            .expect("settings call lock")
            .push((name.to_owned(), access));
        match self.reply {
            SettingsReply::Value => Ok(Some(self.value.clone())),
            SettingsReply::Missing => Ok(None),
            SettingsReply::NotAllowed => Err(SettingsReadError::NotAllowed),
            SettingsReply::Unavailable => Err(SettingsReadError::Unavailable),
        }
    }
}

pub(crate) struct FakeTeams {
    pub(crate) metadata: BTreeMap<String, JsonData>,
    pub(crate) fails: bool,
    pub(crate) lookups: Mutex<Vec<String>>,
}

impl FakeTeams {
    pub(crate) fn new() -> Self {
        Self {
            metadata: BTreeMap::new(),
            fails: false,
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_team(mut self, team_id: &str, metadata: &str) -> Self {
        self.metadata.insert(team_id.to_owned(), json(metadata));
        self
    }

    pub(crate) fn failing() -> Self {
        Self {
            metadata: BTreeMap::new(),
            fails: true,
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn looked_up(&self) -> Vec<String> {
        self.lookups.lock().expect("team lookup lock").clone()
    }
}

#[async_trait]
impl TeamLookup for FakeTeams {
    async fn find_team_metadata(&self, team_id: &str) -> Result<Option<JsonData>, TeamQueryError> {
        self.lookups
            .lock()
            .expect("team lookup lock")
            .push(team_id.to_owned());
        if self.fails {
            return Err(TeamQueryError::Record {
                source: TeamRecordError::NotAnObject,
            });
        }
        Ok(self.metadata.get(team_id).cloned())
    }
}

pub(crate) fn reader(fake: &Arc<FakeSettings>) -> Arc<dyn GeneralSettingsReader> {
    Arc::<FakeSettings>::clone(fake)
}

pub(crate) fn configured(fake: &Arc<FakeConfig>) -> Arc<dyn ConfigEndpointsSource> {
    Arc::<FakeConfig>::clone(fake)
}

pub(crate) fn team_lookup(fake: &Arc<FakeTeams>) -> Arc<dyn TeamLookup> {
    Arc::<FakeTeams>::clone(fake)
}

pub(crate) fn app(
    settings: Option<Arc<dyn GeneralSettingsReader>>,
    config: Arc<dyn ConfigEndpointsSource>,
    teams: Arc<dyn TeamLookup>,
) -> Router {
    routes(PassThroughConfigState {
        settings,
        config_endpoints: config,
        teams,
    })
}

pub(crate) fn read_app(
    settings: &Arc<FakeSettings>,
    config: &Arc<FakeConfig>,
    teams: &Arc<FakeTeams>,
) -> Router {
    app(
        Some(reader(settings)),
        configured(config),
        team_lookup(teams),
    )
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

pub(crate) fn master_key() -> AuthContext {
    master_caller()
}

pub(crate) fn viewer() -> AuthContext {
    caller("user", vec![Role::PlatformViewer])
}

pub(crate) fn member() -> AuthContext {
    caller("user", vec![Role::TeamMember])
}

pub(crate) fn get(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("get request");
    with_caller(request, caller)
}

fn with_caller(mut request: Request<Body>, caller: Option<&AuthContext>) -> Request<Body> {
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    Reply {
        status,
        body: bytes.to_vec(),
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
        serde_json::from_slice(&self.body).expect("json body")
    }

    pub(crate) fn data(&self) -> JsonData {
        self.json()
    }

    pub(crate) fn error(&self) -> ErrorFields {
        self.json::<ErrorBody>().error
    }

    pub(crate) fn detail_error(&self) -> String {
        self.json::<DetailBody>().detail.error
    }
}

pub(crate) fn endpoints_of(reply: &Reply) -> Vec<WireEndpoint> {
    reply.json::<WireResponse>().endpoints
}

pub(crate) fn paths(reply: &Reply) -> Vec<String> {
    endpoints_of(reply)
        .iter()
        .map(|endpoint| endpoint.path.clone())
        .collect()
}

#[derive(Deserialize)]
pub(crate) struct WireResponse {
    pub(crate) endpoints: Vec<WireEndpoint>,
}

#[derive(Deserialize)]
pub(crate) struct WireEndpoint {
    pub(crate) id: Option<String>,
    pub(crate) path: String,
    pub(crate) target: String,
    pub(crate) is_from_config: bool,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorFields,
}

#[derive(Deserialize)]
pub(crate) struct ErrorFields {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) param: Option<String>,
    pub(crate) code: Option<String>,
}

#[derive(Deserialize)]
struct DetailBody {
    detail: DetailFields,
}

#[derive(Deserialize)]
struct DetailFields {
    error: String,
}

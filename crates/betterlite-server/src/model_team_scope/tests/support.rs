#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use betterlite_router::model_group_lookups::error::AccessGroupError;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::db_row_decrypt::warnings::Warnings;
use crate::health_access_and_connections::visibility::{HealthRouter, LookupFailure};
use crate::model_team_scope::lookup::Decryption;
use crate::model_team_scope::reads::{TeamScopeReadError, TeamScopeReads};
use crate::team_access::caller::TeamCaller;

pub(crate) fn caller(user_id: Option<&str>, team_id: Option<&str>) -> TeamCaller {
    TeamCaller {
        user_id: user_id.map(str::to_owned),
        user_role: Some(UserRole::Customer),
        team_id: team_id.map(str::to_owned),
        team_service_account: false,
    }
}

pub(crate) fn admin_caller() -> TeamCaller {
    TeamCaller {
        user_role: Some(UserRole::ProxyAdminViewer),
        ..caller(Some("admin"), Some("team-admin"))
    }
}

pub(crate) fn set(teams: &[&str]) -> BTreeSet<String> {
    teams.iter().map(|team| String::from(*team)).collect()
}

pub(crate) fn object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(String::from(value))
}

pub(crate) fn listed(values: &[&str]) -> Json {
    Json::List(values.iter().map(|value| text(value)).collect())
}

// The fixture deployments' litellm_params, shared by the dumped row and the built spec.
fn default_params() -> JsonObject {
    object(&[("model", text("openai/gpt-4o"))])
}

// One router-shaped row: what get_model_list and the model info responses dump.
pub(crate) fn deployment_json(name: &str, info: &[(&str, Json)]) -> JsonObject {
    object(&[
        ("model_name", text(name)),
        ("litellm_params", Json::Object(default_params())),
        ("model_info", Json::Object(object(info))),
    ])
}

pub(crate) fn model_row(json: &str) -> ProxyModelTable {
    serde_json::from_str(json).expect("a stored model row")
}

// Stored rows by (model_id, model_name), on the fixture provider every deployment uses.
pub(crate) fn stored_rows(entries: &[(&str, &str)]) -> Vec<ProxyModelTable> {
    entries
        .iter()
        .map(|(model_id, model_name)| {
            model_row(&format!(
                r#"{{"model_id":"{model_id}","model_name":"{model_name}","litellm_params":{{"model":"openai/gpt-4o"}}}}"#
            ))
        })
        .collect()
}

pub(crate) fn json_id(row: &JsonObject) -> Option<&str> {
    let info = match row.get("model_info") {
        Some(Json::Object(info)) => info,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => return None,
    };
    match info.get("id") {
        Some(Json::Text(id)) => Some(id),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn name_of(row: &JsonObject) -> &str {
    match row.get("model_name") {
        Some(Json::Text(name)) => name,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}

// The filter's input: each catalog row as get_model_list dumps it.
pub(crate) fn dumped(rows: &[DeploymentSpec]) -> Vec<JsonObject> {
    let mut model_rows = Vec::with_capacity(rows.len());
    for row in rows {
        model_rows.push(row.to_json());
    }
    model_rows
}

pub(crate) fn spec(name: &str, info: &[(&str, Json)]) -> DeploymentSpec {
    DeploymentSpec::new(
        String::from(name),
        default_params(),
        object(info),
        JsonObject::new(),
        String::new,
    )
    .expect("a fixture deployment")
}

// A router whose access-group read fails, as a stale catalog index would.
pub(crate) struct FailingRouter;

impl HealthRouter for FailingRouter {
    fn get_deployment(&self, _model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        Ok(None)
    }

    fn get_model_list(
        &self,
        _model_name: Option<&str>,
        _team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        Ok(Vec::new())
    }

    fn get_model_names(&self, _team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        Ok(Vec::new())
    }

    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        Err(LookupFailure::AccessGroups(
            AccessGroupError::InvalidAccessGroups {
                model_name: String::from("gpt-4o"),
            },
        ))
    }
}

// A deployment whose row lost its id, as LiteLLM's model list allows.
pub(crate) fn unnumbered(name: &str, info: &[(&str, Json)]) -> DeploymentSpec {
    let mut deployment = spec(name, info);
    deployment
        .model_info
        .retain(|field, _| field.as_str() != "id");
    deployment
}

// The stored read a fixture makes fail, one variant per read the scope asks for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Failing {
    User,
    Team,
    Names,
    Row,
}

#[derive(Default)]
pub(crate) struct StubReads {
    pub(crate) users: BTreeMap<String, Vec<String>>,
    pub(crate) teams: BTreeMap<String, Vec<String>>,
    pub(crate) rows: Vec<ProxyModelTable>,
    pub(crate) failing: Vec<Failing>,
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) names_seen: Mutex<Vec<Vec<String>>>,
}

pub(crate) fn read_failure() -> TeamScopeReadError {
    TeamScopeReadError::from(TeamQueryError::from(DbError::NoDbConnected))
}

impl StubReads {
    pub(crate) fn with_users(mut self, users: BTreeMap<String, Vec<String>>) -> Self {
        self.users = users;
        self
    }

    pub(crate) fn with_teams(mut self, teams: BTreeMap<String, Vec<String>>) -> Self {
        self.teams = teams;
        self
    }

    pub(crate) fn with_rows(mut self, rows: Vec<ProxyModelTable>) -> Self {
        self.rows = rows;
        self
    }

    pub(crate) fn with_failing(mut self, failing: Vec<Failing>) -> Self {
        self.failing = failing;
        self
    }

    pub(crate) fn note(&self, label: String) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(label);
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }

    pub(crate) fn names_seen(&self) -> Vec<Vec<String>> {
        let seen = self
            .names_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        seen.clone()
    }
}

#[async_trait]
impl TeamScopeReads for StubReads {
    async fn user_teams(&self, user_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        self.note(format!("user:{user_id}"));
        if self.failing.contains(&Failing::User) {
            return Err(read_failure());
        }
        Ok(self.users.get(user_id).cloned())
    }

    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        self.note(format!("team:{team_id}"));
        if self.failing.contains(&Failing::Team) {
            return Err(read_failure());
        }
        Ok(self.teams.get(team_id).cloned())
    }

    async fn model_ids_by_names(
        &self,
        names: &[String],
    ) -> Result<Vec<String>, TeamScopeReadError> {
        self.note(format!("names:{}", names.join(",")));
        let mut seen = self
            .names_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        seen.push(names.to_vec());
        drop(seen);
        if self.failing.contains(&Failing::Names) {
            return Err(read_failure());
        }
        Ok(self
            .rows
            .iter()
            .filter(|row| names.contains(&row.fields.model_name))
            .map(|row| row.fields.model_id.clone())
            .collect())
    }

    async fn model_row(
        &self,
        model_id: &str,
    ) -> Result<Option<ProxyModelTable>, TeamScopeReadError> {
        self.note(format!("row:{model_id}"));
        if self.failing.contains(&Failing::Row) {
            return Err(read_failure());
        }
        Ok(self
            .rows
            .iter()
            .find(|row| row.fields.model_id == model_id)
            .cloned())
    }
}

// The router reads a filter makes, over a real catalog and B10047's access group cache.
pub(crate) struct FixtureRouter {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    access_groups: AccessGroups,
}

impl FixtureRouter {
    pub(crate) fn new(rows: &[DeploymentSpec]) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for row in rows {
            catalog
                .index_patterns(row)
                .expect("the fixture catalog accepts the row");
            catalog.push(row.clone());
            catalog.model_names.insert(row.model_name.clone());
        }
        Self {
            catalog,
            aliases: AliasMap::default(),
            groups: RoutingGroupIndex::default(),
            access_groups: AccessGroups::default(),
        }
    }

    fn with_lookup<F, T>(&self, read: F) -> T
    where
        F: for<'reader> FnOnce(CatalogLookup<'reader>) -> T,
    {
        let inferred = |_model: &str| -> Option<String> { None };
        read(CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &inferred,
        })
    }
}

impl HealthRouter for FixtureRouter {
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        self.with_lookup(|lookup| {
            let found = lookup.get_deployment(model_id)?;
            Ok(found.cloned())
        })
    }

    fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.with_lookup(|lookup| Ok(lookup.get_model_list(model_name, team_id)?))
    }

    fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        self.with_lookup(|lookup| Ok(lookup.get_model_names(team_id)?))
    }

    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        self.with_lookup(|lookup| {
            Ok(self
                .access_groups
                .get_model_access_groups(&lookup, None, None, None)?)
        })
    }
}

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "a team scope test has no OIDC provider".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct FixedClock;

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

// ProxyConfig's decryption inputs, as the route would build them, with a plaintext fixture key.
pub(crate) struct DecryptFixture {
    pub(crate) cipher: DbCipher,
    pub(crate) secrets: SecretLookup,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) warnings: Warnings,
}

// The fixture cipher: a plaintext signing key, so stored params decrypt without a secret manager.
fn fixture_cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b1216-test-key".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

// The fixture secret lookup: no manager and fixed inputs, so decryption never reaches the network.
fn fixture_secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock),
        oidc_cache: OidcTokenCache::default(),
    }
}

impl DecryptFixture {
    pub(crate) fn new() -> Self {
        Self {
            cipher: fixture_cipher(),
            secrets: fixture_secrets(),
            environment: BTreeMap::new(),
            warnings: Warnings::default(),
        }
    }

    pub(crate) const fn context(&self) -> Decryption<'_> {
        Decryption {
            cipher: &self.cipher,
            secrets: &self.secrets,
            environment: &self.environment,
            warnings: &self.warnings,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

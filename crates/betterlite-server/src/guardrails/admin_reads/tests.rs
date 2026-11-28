#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_guardrails::guardrail_registry::build::GuardrailFactories;
use betterlite_guardrails::guardrail_registry::handler::{
    HandlerSettings, InMemoryGuardrailHandler,
};
use betterlite_guardrails::guardrail_registry::registry::GuardrailRegistry;
use betterlite_guardrails::guardrail_registry::rows::{
    GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError,
};
use betterlite_guardrails::guardrail_startup::runtime::GuardrailRuntime;
use betterlite_guardrails::tool_policy::{RequestToolNames, ToolPolicyLookup};
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use chrono::NaiveDateTime;
use rstest::rstest;
use tower::ServiceExt as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::masking::mask_sensitive_values;
use super::projection::validated;
use super::routes;
use super::state::{ConfigGuardrail, ConfigGuardrails, ReadState, TeamIdsFuture, UserTeams};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::guardrails::form_tables_a::tables_a;
use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::core_tables::core_tables;
use crate::guardrails::provider_forms::spec::{FormModel, FormShape};

const LIST_PATH: &str = "/guardrails/list";
const LIST_V2_PATH: &str = "/v2/guardrails/list";
const SETTINGS_PATH: &str = "/guardrails/ui/add_guardrail_settings";
const FORMS_PATH: &str = "/guardrails/ui/provider_specific_params";
const BEDROCK_PRE: &str = r#"{"guardrail":"bedrock","mode":"pre_call"}"#;
const BEDROCK_SECRET: &str =
    r#"{"guardrail":"bedrock","mode":"pre_call","api_key":"sk-secret-1234"}"#;

fn parse(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

fn object(text: &str) -> JsonObject {
    parse(text).as_object().cloned().expect("an object")
}

fn field<'value>(value: &'value JsonValue, name: &str) -> &'value JsonValue {
    value
        .as_object()
        .and_then(|fields| fields.get(name))
        .unwrap_or_else(|| panic!("the body has no {name}: {value}"))
}

fn text(value: &JsonValue) -> &str {
    value.as_str().expect("a string")
}

fn strings(value: &JsonValue) -> Vec<String> {
    value
        .as_array()
        .expect("a list")
        .iter()
        .map(|item| text(item).to_owned())
        .collect()
}

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn handler() -> InMemoryGuardrailHandler {
    let factories = GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames));
    let settings = HandlerSettings {
        strict_modes: true,
        global_skip_tool_message: false,
    };
    InMemoryGuardrailHandler::new(factories, Box::new(BTreeMap::new()), settings)
}

fn stamp(value: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").expect("a stamp"))
}

fn row(
    id: &str,
    name: &str,
    params: &str,
    info: Option<&str>,
    team_id: Option<&str>,
) -> GuardrailRow {
    let created_at = stamp("2026-01-02T03:04:05");
    let updated_at = stamp("2026-01-02T03:04:05");
    GuardrailRow {
        guardrail_id: id.to_owned(),
        guardrail_name: name.to_owned(),
        litellm_params: Some(params.to_owned()),
        guardrail_info: info.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        status: "active".to_owned(),
        created_at,
        updated_at,
    }
}

fn guardrail(id: Option<&str>, name: &str, params: &str, info: Option<&str>) -> Guardrail {
    let guardrail_id = id.map(str::to_owned);
    let guardrail_name = name.to_owned();
    let litellm_params = decode_litellm_params(params.as_bytes()).expect("the params decode");
    let guardrail_info = info.map(|body| serde_json::from_str(body).expect("the info decodes"));
    Guardrail {
        guardrail_id,
        guardrail_name,
        litellm_params,
        guardrail_info,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    }
}

fn config_entry(
    id: Option<&str>,
    name: &str,
    params: Option<&str>,
    info: Option<&str>,
) -> ConfigGuardrail {
    let guardrail_info = info.map(|body| serde_json::from_str(body).expect("info"));
    ConfigGuardrail {
        guardrail_id: id.map(str::to_owned),
        guardrail_name: name.to_owned(),
        litellm_params: params.map(object),
        guardrail_info,
    }
}

#[derive(Default)]
struct RowStore {
    rows: Vec<GuardrailRow>,
    fail_active: bool,
    fail_find: bool,
    active_reads: usize,
    find_reads: usize,
}

#[derive(Clone, Copy)]
enum Read {
    Active,
    ById,
}

#[derive(Clone, Default)]
struct FakeGuardrails(Arc<Mutex<RowStore>>);

impl FakeGuardrails {
    fn set_rows(&self, rows: Vec<GuardrailRow>) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).rows = rows;
    }

    fn fail_active(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_active = true;
    }

    fn fail_find(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_find = true;
    }

    fn active_reads(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active_reads
    }

    fn find_reads(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .find_reads
    }

    fn read(&self, which: Read) -> Result<Vec<GuardrailRow>, RowsError> {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match which {
            Read::Active => {
                store.active_reads = store.active_reads.saturating_add(1);
                if store.fail_active {
                    return Err(RowsError::MissingRow);
                }
            }
            Read::ById => {
                store.find_reads = store.find_reads.saturating_add(1);
                if store.fail_find {
                    return Err(RowsError::MissingRow);
                }
            }
        }
        Ok(store.rows.clone())
    }
}

impl GuardrailRows for FakeGuardrails {
    fn create(
        &self,
        _row: NewGuardrailRow,
    ) -> impl Future<Output = Result<GuardrailRow, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn delete(&self, _guardrail_id: &str) -> impl Future<Output = Result<(), RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn update(
        &self,
        _guardrail_id: &str,
        _row: GuardrailUpdate,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Err(RowsError::MissingRow))
    }

    fn find_active(&self) -> impl Future<Output = Result<Vec<GuardrailRow>, RowsError>> + Send {
        ready(self.read(Read::Active))
    }

    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        let found = self.read(Read::ById);
        ready(found.map(|rows| {
            rows.into_iter()
                .find(|row| row.guardrail_id == guardrail_id)
        }))
    }

    fn find_by_name(
        &self,
        _guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Ok(None))
    }
}

#[derive(Clone, Default)]
struct FakeConfig(Vec<ConfigGuardrail>);

impl ConfigGuardrails for FakeConfig {
    fn guardrails(&self) -> Vec<ConfigGuardrail> {
        self.0.clone()
    }
}

#[derive(Clone)]
struct FakeTeams {
    outcome: Result<Vec<String>, &'static str>,
    calls: Arc<AtomicUsize>,
}

impl FakeTeams {
    fn new(outcome: Result<Vec<String>, &'static str>) -> Self {
        Self {
            outcome,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl UserTeams for FakeTeams {
    fn team_ids(&self, _user_id: &str) -> TeamIdsFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = match &self.outcome {
            Ok(teams) => Ok(teams.clone()),
            Err(message) => Err(UserLookupError::Unavailable(Box::new(io::Error::other(
                *message,
            )))),
        };
        Box::pin(ready(outcome))
    }
}

struct Setup {
    database: bool,
    rows: Vec<GuardrailRow>,
    config: Vec<ConfigGuardrail>,
    teams: Result<Vec<String>, &'static str>,
    env: BTreeMap<String, String>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            database: false,
            rows: Vec::new(),
            config: Vec::new(),
            teams: Ok(Vec::new()),
            env: BTreeMap::new(),
        }
    }
}

struct Harness {
    router: Router,
    guardrails: FakeGuardrails,
    teams: FakeTeams,
    runtime: Arc<GuardrailRuntime<FakeGuardrails>>,
}

fn harness(setup: Setup) -> Harness {
    let guardrails = FakeGuardrails::default();
    guardrails.set_rows(setup.rows);
    let teams = FakeTeams::new(setup.teams);
    let runtime = Arc::new(GuardrailRuntime::new(
        handler(),
        GuardrailRegistry::new(guardrails.clone()),
    ));
    let state = ReadState {
        runtime: Arc::clone(&runtime),
        database: setup.database,
        config: Arc::new(FakeConfig(setup.config)),
        teams: Arc::new(teams.clone()),
        environment: Arc::new(setup.env),
    };
    Harness {
        router: routes(state),
        guardrails,
        teams,
        runtime,
    }
}

impl Harness {
    async fn seed(&self, guardrail: Guardrail, source: GuardrailDefinitionLocation) {
        let mut held = self.runtime.handler().write().await;
        drop(
            held.initialize_guardrail(guardrail, source)
                .expect("the seed initializes"),
        );
    }

    async fn seed_team_scoped(&self, guardrail: Guardrail, team_id: &str) {
        let id = guardrail.guardrail_id.clone().expect("the seed has an id");
        let mut held = self.runtime.handler().write().await;
        drop(
            held.initialize_guardrail(guardrail.clone(), GuardrailDefinitionLocation::Config)
                .expect("the seed initializes"),
        );
        let mut scoped = guardrail;
        scoped.team_id = Some(team_id.to_owned());
        held.update_in_memory_guardrail(&id, scoped, GuardrailDefinitionLocation::Config)
            .expect("the unchanged params keep the entry");
    }
}

fn principal(subject: &str, roles: Vec<Role>, user: Option<&str>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: user.map(|id| UserIdentity {
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
            source_key: SourceKey::default(),
        },
    }
}

fn admin() -> AuthContext {
    master_caller()
}

fn role_admin() -> AuthContext {
    principal("admin-user", vec![Role::PlatformAdmin], None)
}

fn viewer() -> AuthContext {
    principal("viewer-user", vec![Role::PlatformViewer], None)
}

fn user_caller(id: &str) -> AuthContext {
    principal(id, Vec::new(), Some(id))
}

fn anonymous() -> AuthContext {
    principal("some-key", Vec::new(), None)
}

fn key_caller(role: &str) -> AuthContext {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(&format!(r#"{{"user_role": "{role}"}}"#))
            .expect("the key fixture parses");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs");
    let mut context = principal("key-owner", Vec::new(), None);
    context.principal.source_key = SourceKey(Some(key));
    context
}

async fn get(harness: &Harness, uri: &str, caller: Option<AuthContext>) -> (StatusCode, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(context) = caller {
        builder = builder.extension(context);
    }
    let request = builder.body(Body::empty()).expect("a request");
    let response = harness
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the body is UTF-8"),
    )
}

fn detail(body: &str) -> String {
    text(field(&parse(body), "detail")).to_owned()
}

fn assert_unauthorized(status: StatusCode, body: &str) {
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let parsed = parse(body);
    let error = field(&parsed, "error");
    assert_eq!(text(field(error, "message")), "the API key is not valid");
}

fn entry_ids(body: &JsonValue) -> Vec<String> {
    field(body, "guardrails")
        .as_array()
        .expect("a guardrails list")
        .iter()
        .map(|entry| text(field(entry, "guardrail_id")).to_owned())
        .collect()
}

fn listed_entries(body: &JsonValue) -> &Vec<JsonValue> {
    field(body, "guardrails")
        .as_array()
        .expect("a guardrails list")
}

fn mask_of(text: &str) -> JsonValue {
    JsonValue::Object(mask_sensitive_values(&object(text)))
}

fn nested_secret(depth: usize) -> JsonObject {
    let mut value = JsonValue::String("s3cret-number".to_owned());
    for _ in 0..depth {
        let mut entry = JsonObject::new();
        entry.insert("key".to_owned(), value);
        value = JsonValue::Object(entry);
    }
    value.as_object().cloned().expect("a chain of objects")
}

fn innermost(value: &JsonValue) -> &str {
    let mut current = value;
    while let Some(next) = current.as_object().and_then(|fields| fields.get("key")) {
        current = next;
    }
    current.as_str().expect("a string")
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

fn logged(captured: &Captured) -> String {
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8(bytes).unwrap()
}

include!("tests/masking.rs");
include!("tests/list.rs");
include!("tests/team.rs");
include!("tests/info.rs");
include!("tests/settings.rs");
include!("tests/forms.rs");

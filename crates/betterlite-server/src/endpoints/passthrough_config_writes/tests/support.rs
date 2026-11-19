#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::time::Instant;

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
use betterlite_auth::pass_through_registry::PassThroughMetadata;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::AzureOidcError;
use betterlite_secret_managers::secret_lookup::ports::AzureOidcTokens;
use betterlite_secret_managers::secret_lookup::ports::OidcClock;
use betterlite_secret_managers::secret_lookup::ports::OidcHttp;
use betterlite_secret_managers::secret_lookup::ports::OidcHttpError;
use betterlite_secret_managers::secret_lookup::ports::OidcRequest;
use betterlite_secret_managers::secret_lookup::ports::OidcResponse;
use betterlite_secret_managers::secret_lookup::ports::OidcTokenCache;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::endpoints::passthrough_config_reads::caller::CallerAccess;
use crate::endpoints::passthrough_config_reads::ports::ConfigEndpointsSource;
use crate::endpoints::passthrough_config_reads::ports::GeneralSettingsReader;
use crate::endpoints::passthrough_config_reads::ports::SettingsReadError;
use crate::endpoints::passthrough_config_writes::routes::PassThroughWriteState;
use crate::endpoints::passthrough_config_writes::routes::routes;
use crate::endpoints::passthrough_config_writes::settings_port::ConfigFieldInfo;
use crate::endpoints::passthrough_config_writes::settings_port::ConfigFieldUpdate;
use crate::endpoints::passthrough_config_writes::settings_port::GeneralSettingsError;
use crate::endpoints::passthrough_config_writes::settings_port::GeneralSettingsStore;
use crate::passthrough::route_registration::PassThroughEndpoint;
use crate::passthrough::route_registration::PassthroughParams;
use crate::passthrough::route_registration::SharedPassThroughRoutes;
use crate::passthrough::route_registration::initialize_pass_through_endpoints;

pub(crate) const DB_NOT_CONNECTED: &str = "No DB Connected";

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("test json")
}

pub(crate) fn stored(id: &str, path: &str) -> String {
    stored_with(id, path, "")
}

pub(crate) fn stored_with(id: &str, path: &str, extra: &str) -> String {
    format!(r#"{{"id": "{id}", "path": "{path}", "target": "https://target.example.com"{extra}}}"#)
}

pub(crate) fn entries(items: &[String]) -> JsonData {
    JsonData::Array(items.iter().map(|text| json(text)).collect())
}

pub(crate) struct StoreCells {
    pub(crate) field: JsonData,
    pub(crate) writes: Vec<JsonData>,
}

pub(crate) struct FakeStore {
    pub(crate) cells: Mutex<StoreCells>,
    pub(crate) read_fails: bool,
    pub(crate) write_fails: bool,
}

impl FakeStore {
    pub(crate) fn holding(field: JsonData) -> Self {
        Self {
            cells: Mutex::new(StoreCells {
                field,
                writes: Vec::new(),
            }),
            read_fails: false,
            write_fails: false,
        }
    }

    pub(crate) fn field(&self) -> JsonData {
        lock(&self.cells).field.clone()
    }

    pub(crate) fn stored_ids(&self) -> Vec<String> {
        let JsonData::Array(items) = self.field() else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| {
                let record: HeldEndpoint = serde_json::from_str(&item.to_string()).expect("record");
                record.id.unwrap_or_default()
            })
            .collect()
    }

    pub(crate) fn writes(&self) -> Vec<JsonData> {
        lock(&self.cells).writes.clone()
    }
}

#[derive(Deserialize)]
struct HeldEndpoint {
    id: Option<String>,
}

fn refusal() -> GeneralSettingsError {
    GeneralSettingsError {
        status: StatusCode::BAD_REQUEST,
        detail: DB_NOT_CONNECTED.to_owned(),
    }
}

impl GeneralSettingsStore for FakeStore {
    async fn get_field(&self, name: &str) -> Result<ConfigFieldInfo, GeneralSettingsError> {
        if self.read_fails {
            return Err(refusal());
        }
        Ok(ConfigFieldInfo {
            field_name: name.to_owned(),
            field_value: lock(&self.cells).field.clone(),
        })
    }

    async fn update_field(&self, update: ConfigFieldUpdate) -> Result<(), GeneralSettingsError> {
        if self.write_fails {
            return Err(refusal());
        }
        let mut cells = lock(&self.cells);
        cells.field = update.field_value.clone();
        cells.writes.push(update.field_value);
        Ok(())
    }
}

pub(crate) enum ReaderReply {
    Field(JsonData),
    Missing,
    Fails,
}

pub(crate) struct FakeReader {
    pub(crate) reply: ReaderReply,
}

#[async_trait]
impl GeneralSettingsReader for FakeReader {
    async fn general_setting(
        &self,
        _name: &str,
        _access: CallerAccess,
    ) -> Result<Option<JsonData>, SettingsReadError> {
        match &self.reply {
            ReaderReply::Field(value) => Ok(Some(value.clone())),
            ReaderReply::Missing => Ok(None),
            ReaderReply::Fails => Err(SettingsReadError::Unavailable),
        }
    }
}

pub(crate) struct FakeConfig {
    pub(crate) entries: Option<Vec<JsonData>>,
}

impl FakeConfig {
    pub(crate) fn of(items: &[String]) -> Self {
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            values.push(json(item));
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

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the test has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(crate) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn registry() -> SharedPassThroughRoutes {
    Arc::new(Mutex::new(
        crate::passthrough::route_registration::PassThroughRoutes::default(),
    ))
}

pub(crate) fn lock(
    routes: &SharedPassThroughRoutes,
) -> MutexGuard<'_, crate::passthrough::route_registration::PassThroughRoutes> {
    routes.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn app(
    store: &Arc<FakeStore>,
    registry: &SharedPassThroughRoutes,
    pairs: &[(&str, &str)],
) -> Router {
    routes(PassThroughWriteState {
        settings: Arc::clone(store),
        routes: Arc::clone(registry),
        secrets: Arc::new(secrets()),
        environment: Arc::new(environment(pairs)),
    })
}

pub(crate) fn seeded(id: &str, path: &str) -> PassThroughEndpoint {
    PassThroughEndpoint {
        id: Some(id.to_owned()),
        path: Some(path.to_owned()),
        target: Some("https://old.example.com".to_owned()),
        headers: None,
        forward_headers: None,
        merge_query_params: None,
        default_query_params: None,
        include_subpath: None,
        auth: Some(Json::Bool(true)),
        methods: None,
        cost_per_request: None,
        guardrails: None,
        timeout: None,
    }
}

pub(crate) fn seed(registry: &SharedPassThroughRoutes, items: &[(&str, &str)]) {
    let endpoints: Vec<PassThroughEndpoint> =
        items.iter().map(|(id, path)| seeded(id, path)).collect();
    initialize_pass_through_endpoints(registry, &endpoints, None, &secrets(), &environment(&[]))
        .expect("seed");
}

pub(crate) fn seed_subpath(registry: &SharedPassThroughRoutes, id: &str, path: &str) {
    let mut endpoint = seeded(id, path);
    endpoint.include_subpath = Some(Json::Bool(true));
    initialize_pass_through_endpoints(registry, &[endpoint], None, &secrets(), &environment(&[]))
        .expect("seed");
}

pub(crate) fn registered(registry: &SharedPassThroughRoutes) -> Vec<PassThroughMetadata> {
    lock(registry)
        .registry
        .registrations
        .iter()
        .map(|(_key, metadata)| metadata.clone())
        .collect()
}

pub(crate) fn registered_ids(registry: &SharedPassThroughRoutes) -> Vec<String> {
    registered(registry)
        .into_iter()
        .map(|metadata| metadata.endpoint_id)
        .collect()
}

pub(crate) fn registered_paths(registry: &SharedPassThroughRoutes) -> Vec<String> {
    registered(registry)
        .into_iter()
        .map(|metadata| metadata.path)
        .collect()
}

pub(crate) fn openai_routes(registry: &SharedPassThroughRoutes) -> Vec<String> {
    lock(registry).openai_routes.clone()
}

pub(crate) fn params_of(
    registry: &SharedPassThroughRoutes,
    endpoint_id: &str,
) -> PassthroughParams {
    let prefix = format!("{endpoint_id}:");
    lock(registry)
        .endpoints
        .iter()
        .find(|(key, _params)| key.starts_with(&prefix))
        .map(|(_key, params)| params.clone())
        .expect("a registered endpoint")
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

pub(crate) fn post(uri: &str, body: &str, caller: Option<&AuthContext>) -> Request<Body> {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("post request");
    with_caller(request, caller)
}

pub(crate) fn delete(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    let request = Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
        .expect("delete request");
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

    pub(crate) fn detail_error(&self) -> String {
        self.json::<DetailBody>().detail.error
    }

    pub(crate) fn error(&self) -> ErrorFields {
        self.json::<ErrorBody>().error
    }

    pub(crate) fn issues(&self) -> Vec<WireIssue> {
        self.json::<IssuesBody>().detail
    }
}

pub(crate) fn endpoints_of(reply: &Reply) -> Vec<WireEndpoint> {
    reply.json::<WireResponse>().endpoints
}

pub(crate) fn endpoint_of(reply: &Reply) -> WireEndpoint {
    endpoints_of(reply)
        .into_iter()
        .next()
        .expect("one endpoint")
}

pub(crate) fn endpoint_field(reply: &Reply, name: &str) -> JsonData {
    let body: JsonData = reply.json();
    let JsonData::Object(mut top) = body else {
        panic!("a response object");
    };
    let Some(JsonData::Array(mut items)) = top.remove("endpoints") else {
        panic!("an endpoint list");
    };
    let Some(JsonData::Object(mut record)) = items.into_iter().next() else {
        panic!("an endpoint");
    };
    record.remove(name).expect("the field")
}

pub(crate) fn first_record(store: &FakeStore) -> BTreeMap<String, JsonData> {
    let field = store.field();
    let JsonData::Array(mut items) = field else {
        panic!("a stored list");
    };
    let Some(JsonData::Object(record)) = items.pop() else {
        panic!("a stored record");
    };
    record
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
    pub(crate) auth: bool,
    pub(crate) include_subpath: bool,
    pub(crate) headers: BTreeMap<String, JsonData>,
    pub(crate) timeout: Option<f64>,
    pub(crate) methods: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct DetailBody {
    detail: DetailFields,
}

#[derive(Deserialize)]
struct DetailFields {
    error: String,
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
}

#[derive(Deserialize)]
struct IssuesBody {
    detail: Vec<WireIssue>,
}

#[derive(Deserialize)]
pub(crate) struct WireIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<JsonData>,
    pub(crate) msg: String,
}

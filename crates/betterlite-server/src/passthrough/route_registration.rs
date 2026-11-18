use std::collections::{BTreeMap, BTreeSet};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use betterlite_auth::inference_routes::OPENAI_ROUTES;
use betterlite_auth::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};
use betterlite_models::{decisions::Json, model_registration::JsonObject};
use betterlite_secret_managers::secret_lookup::{SecretLookup, error::SecretLookupError};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::passthrough_route_registry::route_table::RouteTable;

mod tests;

const OS_ENVIRON_PREFIX: &str = "os.environ/";
const LANGFUSE_PUBLIC_KEY: &str = "LANGFUSE_PUBLIC_KEY";
const LANGFUSE_SECRET_KEY: &str = "LANGFUSE_SECRET_KEY";
const AUTHORIZATION: &str = "Authorization";
const DEFAULT_METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "PATCH"];

#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum RouteRegistrationError {
    #[error("Path is required for pass-through endpoint")] PathRequired,
    #[error("auth must be a boolean or a string")] AuthNotBoolean,
    #[error("pass-through headers must set LANGFUSE_PUBLIC_KEY and LANGFUSE_SECRET_KEY together; missing {0}")] MissingLangfuseKey(&'static str),
    #[error("pass-through header {0} must be a string")] HeaderNotText(&'static str),
    #[error("pass-through header {0} references an environment variable that is not set")] HeaderUnresolved(&'static str),
    #[error(transparent)] Secrets(#[from] SecretLookupError),
}

#[rustfmt::skip]
#[derive(Clone, Deserialize)]
pub struct PassThroughEndpoint {
    pub id: Option<String>, pub path: Option<String>, pub target: Option<String>, pub headers: Option<JsonObject>,
    pub forward_headers: Option<bool>, pub merge_query_params: Option<bool>, pub default_query_params: Option<JsonObject>,
    pub include_subpath: Option<Json>, pub auth: Option<Json>, pub methods: Option<Vec<String>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")] pub cost_per_request: Option<Decimal>,
    pub guardrails: Option<JsonObject>, pub timeout: Option<f64>,
}

#[rustfmt::skip]
#[derive(Clone)]
pub struct PassthroughParams {
    pub target: String, pub custom_headers: Option<JsonObject>, pub forward_headers: Option<bool>, pub merge_query_params: Option<bool>,
    pub default_query_params: Option<JsonObject>, pub cost_per_request: Option<Decimal>, pub guardrails: Option<JsonObject>,
    pub timeout: Option<f64>, pub config_file_path: Option<String>,
}

#[rustfmt::skip]
pub struct RouteSpec<'route> {
    pub endpoint_id: &'route str, pub path: &'route str, pub methods: &'route [String], pub auth: bool, pub params: &'route PassthroughParams,
}

#[rustfmt::skip]
pub struct PassThroughRoutes {
    pub table: RouteTable, pub registry: PassThroughRegistry, pub endpoints: BTreeMap<String, PassthroughParams>,
    pub openai_routes: Vec<String>, pub remembered: BTreeMap<(String, BTreeSet<String>), Option<String>>,
}

pub type SharedPassThroughRoutes = Arc<Mutex<PassThroughRoutes>>;

#[rustfmt::skip]
type SecretContext<'context> = (&'context SecretLookup, &'context BTreeMap<String, String>, Option<&'context str>);

#[rustfmt::skip]
impl Default for PassThroughRoutes {
    fn default() -> Self {
        Self { table: RouteTable::default(), registry: PassThroughRegistry { root_path: String::new(), registrations: Vec::new() },
            endpoints: BTreeMap::new(), openai_routes: OPENAI_ROUTES.iter().map(|route| (*route).to_owned()).collect(), remembered: BTreeMap::new() }
    }
}

#[rustfmt::skip]
impl PassThroughRoutes {
    fn install_route(&mut self, spec: &RouteSpec<'_>, route_type: PassThroughRouteType, table_path: String, config_path: Option<&str>) {
        let methods = effective_methods(spec.methods);
        let set: BTreeSet<String> = methods.iter().cloned().collect();
        let fallback = config_path.map(str::to_owned);
        let installed = self.table.add_route_if_not_exists(table_path.clone(), set.clone());
        let stored = if installed {
            self.remembered.insert((table_path, set), fallback.clone());
            fallback
        } else {
            self.table.colliding_routes(&table_path, &set).into_iter()
                .find_map(|route| self.remembered.get(&(route.path.clone(), route.methods.clone())).cloned())
                .unwrap_or(fallback)
        };
        let key = route_key(spec, route_type);
        let mut params = spec.params.clone();
        params.config_file_path = stored;
        self.endpoints.insert(key.clone(), params);
        let metadata = PassThroughMetadata { endpoint_id: spec.endpoint_id.to_owned(), route_type, path: spec.path.to_owned(), methods, auth: Some(spec.auth) };
        match self.registry.registrations.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, existing)) => *existing = metadata,
            None => self.registry.registrations.push((key, metadata)),
        }
    }

    fn remove_stale_entries(&mut self, visited: &BTreeSet<String>) {
        let mut stale = Vec::new();
        for (key, metadata) in take(&mut self.registry.registrations) {
            if visited.contains(&key) {
                self.registry.registrations.push((key, metadata));
            } else {
                self.endpoints.remove(&key);
                stale.push(metadata);
            }
        }
        for metadata in stale { self.remove_openai_routes(&metadata); }
    }

    fn remove_openai_routes(&mut self, removed: &PassThroughMetadata) {
        let base = removed.path.trim_end_matches('/');
        let live = &self.registry.registrations;
        if !live.iter().any(|(_, route)| route.path == removed.path) {
            remove_openai_route(&mut self.openai_routes, &removed.path);
        }
        if removed.route_type != PassThroughRouteType::Subpath {
            return;
        }
        let wildcard = format!("{base}/*");
        let same_base = live.iter().any(|(_, route)| route.route_type == PassThroughRouteType::Subpath && route.path.trim_end_matches('/') == base);
        if !same_base {
            remove_openai_route(&mut self.openai_routes, &wildcard);
        }
    }
}

#[rustfmt::skip]
pub fn add_exact_path_route(routes: &SharedPassThroughRoutes, spec: &RouteSpec<'_>, config_file_path: Option<&str>) {
    let table_path = spec.path.to_owned();
    let mut state = routes.lock().unwrap_or_else(PoisonError::into_inner);
    state.install_route(spec, PassThroughRouteType::Exact, table_path, config_file_path);
}

#[rustfmt::skip]
pub fn add_subpath_route(routes: &SharedPassThroughRoutes, spec: &RouteSpec<'_>, config_file_path: Option<&str>) {
    let table_path = format!("{}/{{subpath:path}}", spec.path);
    let mut state = routes.lock().unwrap_or_else(PoisonError::into_inner);
    state.install_route(spec, PassThroughRouteType::Subpath, table_path, config_file_path);
}

#[rustfmt::skip]
// ast-grep-ignore: no-long-names
pub fn initialize_pass_through_endpoints(
    routes: &SharedPassThroughRoutes, endpoints: &[PassThroughEndpoint], config_file_path: Option<&str>,
    secrets: &SecretLookup, environment: &BTreeMap<String, String>,
) -> Result<(), RouteRegistrationError> {
    let context: SecretContext<'_> = (secrets, environment, config_file_path);
    let mut state = routes.lock().unwrap_or_else(PoisonError::into_inner);
    let mut visited = BTreeSet::new();
    for endpoint in endpoints { register_endpoint(&mut state, endpoint, context, &mut visited)?; }
    state.remove_stale_entries(&visited);
    drop(state);
    Ok(())
}

#[rustfmt::skip]
fn register_endpoint(
    state: &mut PassThroughRoutes, endpoint: &PassThroughEndpoint, context: SecretContext<'_>, visited: &mut BTreeSet<String>,
) -> Result<(), RouteRegistrationError> {
    let (secrets, environment, config_path) = context;
    let Some(path) = endpoint.path.as_deref() else { return Err(RouteRegistrationError::PathRequired); };
    let endpoint_id = endpoint.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let headers = set_env_variables_in_header(secrets, environment, endpoint.headers.as_ref())?;
    let auth = auth_enforced(endpoint.auth.as_ref())?;
    if auth {
        add_openai_route(&mut state.openai_routes, path);
    }
    let Some(target) = endpoint.target.as_deref() else { return Ok(()); };
    let params = PassthroughParams { target: target.to_owned(), custom_headers: headers, forward_headers: endpoint.forward_headers,
        merge_query_params: endpoint.merge_query_params, default_query_params: endpoint.default_query_params.clone(),
        cost_per_request: endpoint.cost_per_request, guardrails: endpoint.guardrails.clone(), timeout: endpoint.timeout, config_file_path: None };
    let methods = effective_methods(endpoint.methods.as_deref().unwrap_or_default());
    let spec = RouteSpec { endpoint_id: &endpoint_id, path, methods: &methods, auth, params: &params };
    state.install_route(&spec, PassThroughRouteType::Exact, path.to_owned(), config_path);
    visited.insert(route_key(&spec, PassThroughRouteType::Exact));
    if matches!(endpoint.include_subpath, Some(Json::Bool(true))) {
        register_subpath(state, &spec, config_path, visited);
    }
    Ok(())
}

#[rustfmt::skip]
fn register_subpath(
    state: &mut PassThroughRoutes, spec: &RouteSpec<'_>, config_path: Option<&str>, visited: &mut BTreeSet<String>,
) {
    if spec.auth {
        add_openai_route(&mut state.openai_routes, &format!("{}/*", spec.path.trim_end_matches('/')));
    }
    state.install_route(spec, PassThroughRouteType::Subpath, format!("{}/{{subpath:path}}", spec.path), config_path);
    visited.insert(route_key(spec, PassThroughRouteType::Subpath));
}

#[rustfmt::skip]
pub fn set_env_variables_in_header(
    secrets: &SecretLookup, environment: &BTreeMap<String, String>, custom_headers: Option<&JsonObject>,
) -> Result<Option<JsonObject>, RouteRegistrationError> {
    let Some(custom) = custom_headers else { return Ok(None); };
    let mut headers = JsonObject::new();
    for (key, value) in custom {
        if key == LANGFUSE_PUBLIC_KEY || key == LANGFUSE_SECRET_KEY {
            let public = langfuse_key(secrets, environment, LANGFUSE_PUBLIC_KEY, custom)?;
            let secret = langfuse_key(secrets, environment, LANGFUSE_SECRET_KEY, custom)?;
            let token = STANDARD.encode(format!("{public}:{secret}"));
            headers.insert(AUTHORIZATION.to_owned(), Json::Text(format!("Basic {token}")));
            continue;
        }
        let resolved = if let Json::Text(text) = value {
            resolved_secret(secrets, environment, text)?.map_or_else(|| value.clone(), Json::Text)
        } else {
            value.clone()
        };
        headers.insert(key.clone(), resolved);
    }
    Ok(Some(headers))
}

#[rustfmt::skip]
fn resolved_secret(
    secrets: &SecretLookup, environment: &BTreeMap<String, String>, text: &str,
) -> Result<Option<String>, RouteRegistrationError> {
    let Some(reference) = text.find(OS_ENVIRON_PREFIX).and_then(|offset| text.get(offset..)) else { return Ok(None); };
    let Some(secret) = secrets.get_secret_str(environment, reference, None)? else { return Ok(None); };
    Ok(Some(text.replace(reference, secret.expose_secret())))
}

#[rustfmt::skip]
fn langfuse_key(
    secrets: &SecretLookup, environment: &BTreeMap<String, String>, name: &'static str, headers: &JsonObject,
) -> Result<String, RouteRegistrationError> {
    let Some(value) = headers.get(name) else { return Err(RouteRegistrationError::MissingLangfuseKey(name)); };
    let Json::Text(text) = value else { return Err(RouteRegistrationError::HeaderNotText(name)); };
    if !text.starts_with(OS_ENVIRON_PREFIX) {
        return Ok(text.clone());
    }
    let Some(secret) = secrets.get_secret_str(environment, text, None)? else { return Err(RouteRegistrationError::HeaderUnresolved(name)); };
    Ok(secret.expose_secret().to_owned())
}

#[rustfmt::skip]
fn auth_enforced(auth: Option<&Json>) -> Result<bool, RouteRegistrationError> {
    match auth {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(enforced)) => Ok(*enforced),
        Some(Json::Text(text)) => Ok(text.to_lowercase() == "true"),
        Some(Json::Number(_) | Json::List(_) | Json::Object(_)) => Err(RouteRegistrationError::AuthNotBoolean),
    }
}

#[rustfmt::skip]
fn add_openai_route(routes: &mut Vec<String>, path: &str) {
    if !routes.iter().any(|existing| existing.as_str() == path) { routes.push(path.to_owned()); }
}

#[rustfmt::skip]
fn remove_openai_route(routes: &mut Vec<String>, route: &str) {
    if OPENAI_ROUTES.contains(&route) { return; }
    if let Some(position) = routes.iter().position(|existing| existing.as_str() == route) { routes.remove(position); }
}

#[rustfmt::skip]
fn route_key(spec: &RouteSpec<'_>, route_type: PassThroughRouteType) -> String {
    let type_name = match route_type {
        PassThroughRouteType::Exact => "exact",
        PassThroughRouteType::Subpath => "subpath",
    };
    let mut methods = effective_methods(spec.methods);
    methods.sort_unstable();
    format!("{}:{type_name}:{}:{}", spec.endpoint_id, spec.path, methods.join(","))
}

#[rustfmt::skip]
fn effective_methods(methods: &[String]) -> Vec<String> {
    if methods.is_empty() { return DEFAULT_METHODS.iter().map(|method| (*method).to_owned()).collect(); }
    methods.to_vec()
}

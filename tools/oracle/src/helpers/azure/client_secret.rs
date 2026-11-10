use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_client_secret::authority::{self, ConfiguredRegion};
use betterlite_azure::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use betterlite_azure::azure_client_secret::transport::{
    Clock, HttpMethod, HttpReply, HttpRequest, Transport,
};
use betterlite_azure::azure_client_secret::{
    ClientSecretError, ClientSecretInput, ResolvedCredentials, SecretResolver,
    resolve_client_secret,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_client_secret::Settings;
use betterlite_openai::error::{ErrorRequest, ErrorResponse, ProviderError, ProviderErrorParts};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const NOW: i64 = 1_700_000_000;
const FIXTURE_TENANT: &str = "tenant";
const FIXTURE_CLIENT: &str = "client";
const FIXTURE_SECRET: &str = "secret";

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    ClientSecretForm(GrantSteps),
    TenantScope(GrantSteps),
    OauthFailure(GrantSteps),
    TokenExpiry(GrantSteps),
    #[serde(rename = "client_secret_reference_resolution")]
    SecretReferenceResolution(ResolveSteps),
    ClientSecretProviderCache(CacheSteps),
    AzureErrorFields(ErrorSteps),
    RegionalAuthorityInputs(RegionSteps),
    RegionalAuthorityDiscovery(RegionSteps),
    RegionalAuthorityHost(RegionSteps),
}

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct GrantSteps {
    steps: Vec<GrantStep>,
}

#[derive(Deserialize)]
struct GrantStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    tenant_id: String,
    client_id: String,
    client_secret: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    authority_host: Option<String>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Clone, Deserialize)]
struct Reply {
    #[serde(default)]
    url_contains: Option<String>,
    status: u16,
    body: String,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ResolveSteps {
    steps: Vec<ResolveStep>,
}

#[derive(Deserialize)]
struct ResolveStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    tenant_id: String,
    client_id: String,
    client_secret: String,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CacheSteps {
    steps: Vec<CacheStep>,
}

#[derive(Deserialize)]
struct CacheStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    calls: Vec<CacheCall>,
}

#[derive(Deserialize)]
struct CacheCall {
    tenant_id: String,
    client_id: String,
    client_secret: String,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ErrorSteps {
    steps: Vec<ErrorStep>,
}

#[derive(Deserialize)]
struct ErrorStep {
    status_code: u16,
    message: String,
    #[serde(default)]
    headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    request: Option<RequestFixture>,
    #[serde(default)]
    response: Option<ResponseFixture>,
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Deserialize)]
struct RequestFixture {
    method: String,
    url: String,
}

#[derive(Deserialize)]
struct ResponseFixture {
    status_code: u16,
}

#[derive(Deserialize)]
pub(crate) struct RegionSteps {
    steps: Vec<RegionStep>,
}

#[derive(Deserialize)]
struct RegionStep {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    replies: Vec<Reply>,
}

struct FixtureTransport {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FixtureTransport {
    fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            let index = replies
                .iter()
                .position(|reply| {
                    reply
                        .url_contains
                        .as_ref()
                        .is_none_or(|needle| request.url.contains(needle))
                })
                .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
            replies
                .remove(index)
                .ok_or_else(|| io::Error::other("no reply"))?
        };
        match reply.error {
            Some(message) => Err(io::Error::other(message)),
            None => Ok(HttpReply {
                status: reply.status,
                body: reply.body,
            }),
        }
    }
}

struct FixtureClock {
    now: i64,
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

struct FixtureResolver {
    environment: BTreeMap<String, String>,
}

// the B8500 read a resolved reference gets: an os.environ/ name reads the map, and true/false read back as unset
impl SecretResolver for FixtureResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        let name = reference.replace("os.environ/", "");
        let text = self.environment.get(&name)?;
        let lowered = text.trim().to_lowercase();
        if lowered == "true" || lowered == "false" {
            return None;
        }
        Some(SecretString::from(text.clone()))
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn load_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map(|config| config.sections.azure_client_secret)
    .map_err(|error| failure("ConfigError", error.to_string()))
}

fn input_of(
    tenant_id: &str,
    client_id: &str,
    client_secret: &str,
    scope: Option<&str>,
) -> ClientSecretInput {
    ClientSecretInput {
        tenant_id: tenant_id.to_owned(),
        client_id: client_id.to_owned(),
        client_secret: SecretString::from(client_secret),
        scope: scope.map(str::to_owned),
    }
}

fn pairs(pairs: &[(String, String)]) -> Value {
    let map: BTreeMap<&str, &str> = pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    json!(map)
}

fn request_json(request: &HttpRequest) -> Value {
    json!({
        "method": match request.method {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
        },
        "url": request.url,
        "query": pairs(&request.query),
        "form": pairs(&request.form),
    })
}

fn provider_for(
    cache: &mut ProviderCache,
    input: &ClientSecretInput,
    resolver: &FixtureResolver,
    authority_host: Option<&str>,
) -> Result<Arc<ClientSecretProvider>, Box<HelperError>> {
    cache
        .provider(input, resolver, authority_host)
        .map_err(|error| failure("ClientAuthenticationError", error.to_string()))
}

fn grant_value(transport: &FixtureTransport, token: &str) -> Value {
    let post = transport
        .requests()
        .into_iter()
        .find(|request| request.method == HttpMethod::Post);
    match post {
        Some(request) => json!({
            "kind": "token",
            "token_url": request.url,
            "form": pairs(&request.form),
            "token": token,
        }),
        None => json!({"kind": "error"}),
    }
}

fn grant_step(step: &GrantStep) -> Result<Value, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let cache_transport = Arc::clone(&transport);
    let mut cache = ProviderCache::new(
        cache_transport,
        Arc::new(FixtureClock { now: NOW }),
        settings,
    );
    let input = input_of(
        &step.tenant_id,
        &step.client_id,
        &step.client_secret,
        step.scope.as_deref(),
    );
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let Ok(provider) = provider_for(
        &mut cache,
        &input,
        &resolver,
        step.authority_host.as_deref(),
    ) else {
        return Ok(json!({"kind": "error"}));
    };
    match provider.token() {
        Ok(token) => Ok(grant_value(&transport, token.token.expose_secret())),
        Err(_error) => Ok(json!({"kind": "error"})),
    }
}

fn expiry_step(step: &GrantStep) -> Result<Value, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let cache_transport = Arc::clone(&transport);
    let mut cache = ProviderCache::new(
        cache_transport,
        Arc::new(FixtureClock { now: NOW }),
        settings,
    );
    let input = input_of(
        &step.tenant_id,
        &step.client_id,
        &step.client_secret,
        step.scope.as_deref(),
    );
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let provider = provider_for(
        &mut cache,
        &input,
        &resolver,
        step.authority_host.as_deref(),
    )?;
    let token = provider
        .token()
        .map_err(|error| failure("ClientAuthenticationError", error.to_string()))?;
    Ok(json!({
        "expires_on": token.expires_on,
        "token_type": token.token_type,
        "refresh_on": token.refresh_on,
    }))
}

fn resolve_step(step: &ResolveStep) -> Result<Value, Box<HelperError>> {
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let input = input_of(
        &step.tenant_id,
        &step.client_id,
        &step.client_secret,
        step.scope.as_deref(),
    );
    match resolve_client_secret(&input, &resolver) {
        Ok(credentials) => Ok(resolved_value(&credentials)),
        Err(ClientSecretError::MissingCredentials) => Ok(json!({"kind": "error"})),
        Err(error) => Err(failure("ClientAuthenticationError", error.to_string())),
    }
}

fn resolved_value(credentials: &ResolvedCredentials) -> Value {
    json!({
        "kind": "token",
        "tenant_id": credentials.tenant_id,
        "client_id": credentials.client_id,
        "client_secret": credentials.client_secret.expose_secret(),
        "scope": credentials.scope,
    })
}

fn identity(call: &CacheCall) -> ClientSecretInput {
    input_of(
        &call.tenant_id,
        &call.client_id,
        &call.client_secret,
        call.scope.as_deref(),
    )
}

fn cache_steps(steps: &[CacheStep]) -> Result<Value, Box<HelperError>> {
    let mut cache = ProviderCache::new(
        Arc::new(FixtureTransport::new(&[])),
        Arc::new(FixtureClock { now: NOW }),
        Settings::default(),
    );
    let mut seen: Vec<Arc<ClientSecretProvider>> = Vec::new();
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let resolver = FixtureResolver {
            environment: step.environment.clone(),
        };
        results.push(cache_step(step, &mut cache, &resolver, &mut seen)?);
    }
    Ok(Value::Array(results))
}

fn cache_step(
    step: &CacheStep,
    cache: &mut ProviderCache,
    resolver: &FixtureResolver,
    seen: &mut Vec<Arc<ClientSecretProvider>>,
) -> Result<Value, Box<HelperError>> {
    let mut constructed = 0_usize;
    let mut providers: Vec<Arc<ClientSecretProvider>> = Vec::with_capacity(step.calls.len());
    for call in &step.calls {
        let provider = provider_for(cache, &identity(call), resolver, None)?;
        if !seen.iter().any(|known| Arc::ptr_eq(known, &provider)) {
            seen.push(Arc::clone(&provider));
            constructed = constructed.saturating_add(1);
        }
        providers.push(provider);
    }
    let same = providers
        .first()
        .zip(providers.last())
        .is_some_and(|(first, last)| Arc::ptr_eq(first, last));
    Ok(json!({"constructed": constructed, "same": same}))
}

fn error_body(
    step: &ErrorStep,
) -> Result<Option<serde_json::Map<String, Value>>, Box<HelperError>> {
    match &step.body {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map.clone())),
        Some(_other) => Err(failure("InvalidInput", "body must be an object".to_owned())),
    }
}

fn error_step(step: &ErrorStep) -> Result<Value, Box<HelperError>> {
    let request = step.request.as_ref().map(|request| ErrorRequest {
        method: request.method.clone(),
        url: request.url.clone(),
    });
    let response = step.response.as_ref().map(|response| ErrorResponse {
        status_code: response.status_code,
        request: request.clone().unwrap_or_else(|| ErrorRequest {
            method: String::new(),
            url: String::new(),
        }),
    });
    let error = ProviderError::from(ProviderErrorParts {
        status_code: step.status_code,
        message: step.message.clone(),
        headers: step
            .headers
            .as_ref()
            .map(|headers| headers.clone().into_iter().collect()),
        request,
        response,
        body: error_body(step)?,
        status_code_is_synthesized: false,
    });
    Ok(json!({
        "status_code": error.status_code,
        "message": error.message,
        "request": {"method": error.request.method, "url": error.request.url},
        "response": {
            "status_code": error.response.status_code,
            "request": {"method": error.response.request.method, "url": error.response.request.url},
        },
        "headers": pairs(&error.headers.unwrap_or_default()),
        "body": error.body,
        "status_code_is_synthesized": error.status_code_is_synthesized,
    }))
}

fn configured_json(configured: Option<&ConfiguredRegion>) -> Value {
    match configured {
        None => Value::Null,
        Some(ConfiguredRegion::AutoDetect) => json!("auto"),
        Some(ConfiguredRegion::Named(name)) => json!(name),
    }
}

fn configured_input(settings: &Settings) -> Value {
    settings
        .azure_regional_authority_name
        .as_deref()
        .map_or(Value::Null, |name| {
            configured_json(Some(&authority::configured_region(name)))
        })
}

fn imds_report(transport: &FixtureTransport) -> (usize, Value) {
    let requests: Vec<HttpRequest> = transport
        .requests()
        .into_iter()
        .filter(|request| request.url.contains("169.254.169.254"))
        .collect();
    let metadata = requests
        .first()
        .and_then(|request| {
            request
                .headers
                .iter()
                .find(|(name, _value)| name == "Metadata")
                .map(|(_name, value)| value.clone())
        })
        .map_or(Value::Null, |value| json!(value));
    (requests.len(), metadata)
}

fn regional_host(requests: &[HttpRequest], environment: &BTreeMap<String, String>) -> Value {
    let configured = environment
        .get("AZURE_AUTHORITY_HOST")
        .cloned()
        .unwrap_or_else(|| "login.microsoftonline.com".to_owned());
    let central = configured
        .trim_start_matches("https://")
        .trim_end_matches([' ', '/'])
        .to_owned();
    let discoveries: Vec<&HttpRequest> = requests
        .iter()
        .filter(|request| request.url.contains(".well-known/openid-configuration"))
        .collect();
    let Some(last) = discoveries.last() else {
        return Value::Null;
    };
    let host = last
        .url
        .split_once("://")
        .and_then(|(_scheme, rest)| rest.split('/').next())
        .unwrap_or_default();
    if host == central {
        Value::Null
    } else {
        json!(host)
    }
}

fn flow_requests(step: &RegionStep) -> Result<Vec<HttpRequest>, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let cache_transport = Arc::clone(&transport);
    let mut cache = ProviderCache::new(
        cache_transport,
        Arc::new(FixtureClock { now: NOW }),
        settings,
    );
    let input = input_of(FIXTURE_TENANT, FIXTURE_CLIENT, FIXTURE_SECRET, None);
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let authority_host = step
        .environment
        .get("AZURE_AUTHORITY_HOST")
        .map(String::as_str);
    let provider = provider_for(&mut cache, &input, &resolver, authority_host)?;
    provider
        .token()
        .map_err(|error| failure("ClientAuthenticationError", error.to_string()))?;
    Ok(transport.requests())
}

fn input_report(step: &RegionStep) -> Result<Value, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let selection = authority::select_region(&settings, &FixtureTransport::new(&step.replies));
    let requests = flow_requests(step)?;
    Ok(json!({
        "configured": configured_input(&settings),
        "region": selection.region,
        "warnings": selection.warnings,
        "debug": selection.region_debug,
        "regional_host": regional_host(&requests, &step.environment),
    }))
}

fn discovery_report(step: &RegionStep) -> Result<Value, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let selection_transport = FixtureTransport::new(&step.replies);
    let selection = authority::select_region(&settings, &selection_transport);
    let (imds_requests, metadata) = imds_report(&selection_transport);
    let requests = flow_requests(step)?;
    Ok(json!({
        "region": selection.region,
        "warnings": selection.warnings,
        "debug": selection.region_debug,
        "imds_requests": imds_requests,
        "metadata": metadata,
        "regional_host": regional_host(&requests, &step.environment),
    }))
}

fn host_value(
    transport: &FixtureTransport,
    outcome: &str,
    environment: &BTreeMap<String, String>,
) -> Value {
    let requests = transport.requests();
    json!({
        "outcome": outcome,
        "requests": Value::Array(requests.iter().map(request_json).collect()),
        "regional_host": regional_host(&requests, environment),
    })
}

fn host_report(step: &RegionStep) -> Result<Value, Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let cache_transport = Arc::clone(&transport);
    let mut cache = ProviderCache::new(
        cache_transport,
        Arc::new(FixtureClock { now: NOW }),
        settings,
    );
    let input = input_of(FIXTURE_TENANT, FIXTURE_CLIENT, FIXTURE_SECRET, None);
    let resolver = FixtureResolver {
        environment: step.environment.clone(),
    };
    let authority_host = step
        .environment
        .get("AZURE_AUTHORITY_HOST")
        .map(String::as_str);
    let Ok(provider) = provider_for(&mut cache, &input, &resolver, authority_host) else {
        return Ok(host_value(&transport, "error", &step.environment));
    };
    let outcome = if provider.token().is_ok() {
        "token"
    } else {
        "error"
    };
    Ok(host_value(&transport, outcome, &step.environment))
}

fn collect_steps<T, F>(steps: &[T], report: F) -> HelperResult
where
    F: Fn(&T) -> Result<Value, Box<HelperError>>,
{
    steps
        .iter()
        .map(report)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()
        .map(Value::Array)
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ClientSecretForm(steps)
            | Input::TenantScope(steps)
            | Input::OauthFailure(steps) => collect_steps(&steps.steps, grant_step),
            Input::TokenExpiry(steps) => collect_steps(&steps.steps, expiry_step),
            Input::SecretReferenceResolution(steps) => collect_steps(&steps.steps, resolve_step),
            Input::ClientSecretProviderCache(steps) => cache_steps(&steps.steps),
            Input::AzureErrorFields(steps) => collect_steps(&steps.steps, error_step),
            Input::RegionalAuthorityInputs(steps) => collect_steps(&steps.steps, input_report),
            Input::RegionalAuthorityDiscovery(steps) => {
                collect_steps(&steps.steps, discovery_report)
            }
            Input::RegionalAuthorityHost(steps) => collect_steps(&steps.steps, host_report),
        }
    }
}

mod tests;

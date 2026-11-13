use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use betterlite_azure::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use betterlite_azure::azure_client_secret::token::AccessToken;
use betterlite_azure::azure_client_secret::transport::HttpMethod;
use betterlite_azure::azure_client_secret::{
    ClientSecretError, ClientSecretInput, ResolvedCredentials, resolve_client_secret,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_client_secret::Settings;
use betterlite_openai::error::{ErrorRequest, ErrorResponse, ProviderError, ProviderErrorParts};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};
use fixture::{FixtureClock, FixtureResolver, FixtureTransport, NOW, failure, input_of, pairs};
use steps::{Call, Step};

mod fixture;
mod region;
mod steps;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    ClientSecretForm(steps::Steps),
    TenantScope(steps::Steps),
    OauthFailure(steps::Steps),
    TokenExpiry(steps::Steps),
    #[serde(rename = "client_secret_reference_resolution")]
    SecretReferenceResolution(steps::Steps),
    ClientSecretProviderCache(steps::Steps),
    AzureErrorFields(steps::Steps),
    RegionalAuthorityInputs(steps::Steps),
    RegionalAuthorityDiscovery(steps::Steps),
    RegionalAuthorityHost(steps::Steps),
}

fn load_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map(|config| config.sections.azure_client_secret)
    .map_err(|error| failure("ConfigError", error.to_string()))
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

fn step_cache(
    step: &Step,
    settings: Settings,
) -> (Arc<FixtureTransport>, Arc<FixtureClock>, ProviderCache) {
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let clock = Arc::new(FixtureClock {
        now: AtomicI64::new(NOW),
    });
    let cache_transport = Arc::clone(&transport);
    let cache_clock = Arc::clone(&clock);
    let cache = ProviderCache::new(cache_transport, cache_clock, settings);
    (transport, clock, cache)
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

type Attempts = (Arc<FixtureTransport>, Vec<Option<AccessToken>>);

// one token request, then a second after the clock moved on when the step asks for it
fn attempts(step: &Step) -> Result<Attempts, Box<HelperError>> {
    let (transport, clock, mut cache) = step_cache(step, load_settings(&step.environment)?);
    let resolver = FixtureResolver::from(step);
    let host = step.authority_host.as_deref();
    let provider = provider_for(&mut cache, &input_of(step), &resolver, host).ok();
    let mut tokens = Vec::new();
    for offset in std::iter::once(0).chain(step.advance) {
        let later = NOW.saturating_add(offset);
        clock.now.store(later, Ordering::SeqCst);
        tokens.push(provider.as_ref().and_then(|ready| ready.token().ok()));
    }
    Ok((transport, tokens))
}

fn grant_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let (transport, tokens) = attempts(step)?;
    Ok(match tokens.into_iter().next().flatten() {
        Some(token) => grant_value(&transport, token.token.expose_secret()),
        None => json!({"kind": "error"}),
    })
}

fn expiry_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let (_transport, tokens) = attempts(step)?;
    let mut values = Vec::new();
    for token in tokens {
        values.push(match token {
            Some(acquired) => json!({
                "expires_on": acquired.expires_on,
                "token_type": acquired.token_type,
                "refresh_on": acquired.refresh_on,
            }),
            None => json!({"kind": "error"}),
        });
    }
    Ok(Value::Array(values))
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

fn resolve_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let resolver = FixtureResolver::from(step);
    match resolve_client_secret(&input_of(step), &resolver) {
        Ok(credentials) => Ok(resolved_value(&credentials)),
        Err(ClientSecretError::MissingCredentials) => Ok(json!({"kind": "error"})),
        Err(error) => Err(failure("CredentialError", error.to_string())),
    }
}

fn call_input(call: &Call) -> ClientSecretInput {
    ClientSecretInput {
        tenant_id: call.tenant_id.clone(),
        client_id: call.client_id.clone(),
        client_secret: SecretString::from(call.client_secret.clone()),
        scope: call.scope.clone(),
    }
}

fn cache_report(steps: &[Step]) -> HelperResult {
    let mut cache = ProviderCache::new(
        Arc::new(FixtureTransport::new(&[])),
        Arc::new(FixtureClock {
            now: AtomicI64::new(NOW),
        }),
        Settings::default(),
    );
    let mut seen: Vec<Arc<ClientSecretProvider>> = Vec::new();
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        results.push(cache_step(step, &mut cache, &mut seen)?);
    }
    Ok(Value::Array(results))
}

fn cache_step(
    step: &Step,
    cache: &mut ProviderCache,
    seen: &mut Vec<Arc<ClientSecretProvider>>,
) -> Result<Value, Box<HelperError>> {
    let resolver = FixtureResolver::from(step);
    let mut constructed = 0_usize;
    let mut providers: Vec<Arc<ClientSecretProvider>> = Vec::with_capacity(step.calls.len());
    for call in &step.calls {
        let provider = provider_for(cache, &call_input(call), &resolver, None)?;
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

fn error_body(step: &Step) -> Result<Option<serde_json::Map<String, Value>>, Box<HelperError>> {
    match &step.body {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map.clone())),
        Some(_other) => Err(failure("InvalidInput", "body must be an object".to_owned())),
    }
}

fn error_step(step: &Step) -> Result<Value, Box<HelperError>> {
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
        "headers": error.headers.as_deref().map(pairs),
        "body": error.body,
        "status_code_is_synthesized": error.status_code_is_synthesized,
    }))
}

fn collect_steps<F>(steps: &[Step], report: F) -> HelperResult
where
    F: Fn(&Step) -> Result<Value, Box<HelperError>>,
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
            Input::ClientSecretProviderCache(steps) => cache_report(&steps.steps),
            Input::AzureErrorFields(steps) => collect_steps(&steps.steps, error_step),
            Input::RegionalAuthorityInputs(steps) => {
                collect_steps(&steps.steps, region::inputs_report)
            }
            Input::RegionalAuthorityDiscovery(steps) => {
                collect_steps(&steps.steps, region::discovery_report)
            }
            Input::RegionalAuthorityHost(steps) => collect_steps(&steps.steps, region::host_report),
        }
    }
}

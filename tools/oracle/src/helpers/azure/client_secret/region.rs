use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use betterlite_azure::azure_client_secret::ClientSecretInput;
use betterlite_azure::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use betterlite_azure::azure_client_secret::region::{self, ConfiguredRegion};
use betterlite_azure::azure_client_secret::transport::HttpRequest;
use betterlite_config::sections::azure_client_secret::Settings;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::fixture::{FixtureClock, FixtureResolver, FixtureTransport, NOW, failure, request_json};
use super::steps::Step;
use super::{load_settings, provider_for};
use crate::helpers::HelperError;

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
            configured_json(Some(&region::configured_region(name)))
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

fn prepare(
    step: &Step,
) -> Result<(Arc<FixtureTransport>, ProviderCache, FixtureResolver), Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = Arc::new(FixtureTransport::new(&step.replies));
    let cache_transport = Arc::clone(&transport);
    let cache = ProviderCache::new(
        cache_transport,
        Arc::new(FixtureClock {
            now: AtomicI64::new(NOW),
        }),
        settings,
    );
    let resolver = FixtureResolver::from(step);
    Ok((transport, cache, resolver))
}

fn connect(
    step: &Step,
    cache: &mut ProviderCache,
    resolver: &FixtureResolver,
) -> Result<Arc<ClientSecretProvider>, Box<HelperError>> {
    let authority_host = step
        .environment
        .get("AZURE_AUTHORITY_HOST")
        .map(String::as_str);
    // the region cases carry no credentials, the LiteLLM side signs in as tenant, client and secret
    let input = ClientSecretInput {
        tenant_id: "tenant".to_owned(),
        client_id: "client".to_owned(),
        client_secret: SecretString::from("secret"),
        scope: None,
    };
    provider_for(cache, &input, resolver, authority_host)
}

fn flow_requests(step: &Step) -> Result<Vec<HttpRequest>, Box<HelperError>> {
    let (transport, mut cache, resolver) = prepare(step)?;
    connect(step, &mut cache, &resolver)?
        .token()
        .map_err(|error| failure("ClientAuthenticationError", error.to_string()))?;
    Ok(transport.requests())
}

fn selection_report(
    step: &Step,
) -> Result<(Settings, region::RegionSelection, usize, Value, Value), Box<HelperError>> {
    let settings = load_settings(&step.environment)?;
    let transport = FixtureTransport::new(&step.replies);
    let selection = region::select_region(&settings, &transport);
    let (imds_requests, metadata) = imds_report(&transport);
    let host = regional_host(&flow_requests(step)?, &step.environment);
    Ok((settings, selection, imds_requests, metadata, host))
}

fn selection_value(selection: &region::RegionSelection, extra: &[(&str, Value)]) -> Value {
    let mut fields: BTreeMap<&str, Value> = BTreeMap::from([
        ("region", json!(selection.region)),
        ("warnings", json!(selection.warnings)),
        ("debug", json!(selection.region_debug)),
    ]);
    fields.extend(extra.iter().map(|(name, value)| (*name, value.clone())));
    json!(fields)
}

pub(super) fn inputs_report(step: &Step) -> Result<Value, Box<HelperError>> {
    let (settings, selection, _imds_requests, _metadata, host) = selection_report(step)?;
    let mut extra = vec![("configured", configured_input(&settings))];
    extra.push(("regional_host", host));
    Ok(selection_value(&selection, &extra))
}

pub(super) fn discovery_report(step: &Step) -> Result<Value, Box<HelperError>> {
    let (_settings, selection, imds_requests, metadata, host) = selection_report(step)?;
    let mut extra = vec![("imds_requests", json!(imds_requests))];
    extra.extend([("metadata", metadata), ("regional_host", host)]);
    Ok(selection_value(&selection, &extra))
}

pub(super) fn host_report(step: &Step) -> Result<Value, Box<HelperError>> {
    let (transport, mut cache, resolver) = prepare(step)?;
    let outcome = match connect(step, &mut cache, &resolver) {
        Ok(provider) => {
            if provider.token().is_ok() {
                "token"
            } else {
                "error"
            }
        }
        Err(_error) => "error",
    };
    let requests = transport.requests();
    Ok(json!({
        "outcome": outcome,
        "requests": Value::Array(requests.iter().map(request_json).collect()),
        "regional_host": regional_host(&requests, &step.environment),
    }))
}

use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::batch::logger::{BATCH_SIZE_VARIABLE, BatchSettings, FLUSH_INTERVAL_VARIABLE};

use super::error::SentinelError;

pub const DEFAULT_STREAM_NAME: &str = "Custom-LiteLLM";
pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";
pub const DEFAULT_MONITOR_SCOPE: &str = "https://monitor.azure.com/.default";
const GOVERNMENT_MONITOR_SCOPE: &str = "https://monitor.azure.us/.default";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SentinelInit {
    pub dcr_immutable_id: Option<String>,
    pub stream_name: Option<String>,
    pub endpoint: Option<String>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub audit_stream_name: Option<String>,
    pub authority_host: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SentinelSettings {
    pub model_url: String,
    pub audit_url: String,
    pub token_url: String,
    pub scope: &'static str,
    pub client_id: String,
    pub client_secret: SecretString,
    pub batch: BatchSettings,
}

// the first non-empty value among the argument and the variables, as LiteLLM's `or` chain reads them
fn first_set(
    argument: Option<&str>,
    environment: &dyn EnvironmentLookup,
    names: &[&str],
) -> Option<String> {
    argument
        .map(str::to_owned)
        .into_iter()
        .chain(names.iter().filter_map(|name| environment.lookup(name)))
        .find(|value| !value.is_empty())
}

#[must_use]
pub fn normalize_authority_host(authority_host: &str) -> String {
    let stripped = authority_host.trim().trim_end_matches('/');
    if stripped.contains("://") {
        stripped.to_owned()
    } else {
        format!("https://{stripped}")
    }
}

#[must_use]
pub fn resolve_oauth_scope(authority_host: &str) -> &'static str {
    let host = Url::parse(authority_host)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned));
    match host.as_deref() {
        Some("login.microsoftonline.us") => GOVERNMENT_MONITOR_SCOPE,
        _ => DEFAULT_MONITOR_SCOPE,
    }
}

fn api_endpoint(endpoint: &str, dcr_immutable_id: &str, stream_name: &str) -> String {
    format!(
        "{}/dataCollectionRules/{dcr_immutable_id}/streams/{stream_name}?api-version=2023-01-01",
        endpoint.trim_end_matches('/')
    )
}

struct Credentials {
    dcr_immutable_id: String,
    endpoint: String,
    tenant_id: String,
    client_id: String,
    client_secret: String,
}

// the checks run in LiteLLM's order, so the first missing value names the error
fn credentials(
    init: &SentinelInit,
    environment: &dyn EnvironmentLookup,
) -> Result<Credentials, SentinelError> {
    let first = |argument: &Option<String>, names: &[&str]| {
        first_set(argument.as_deref(), environment, names)
    };
    Ok(Credentials {
        dcr_immutable_id: first(&init.dcr_immutable_id, &["AZURE_SENTINEL_DCR_IMMUTABLE_ID"])
            .ok_or(SentinelError::MissingDcrImmutableId)?,
        endpoint: first(&init.endpoint, &["AZURE_SENTINEL_ENDPOINT"])
            .ok_or(SentinelError::MissingEndpoint)?,
        tenant_id: first(
            &init.tenant_id,
            &["AZURE_SENTINEL_TENANT_ID", "AZURE_TENANT_ID"],
        )
        .ok_or(SentinelError::MissingTenantId)?,
        client_id: first(
            &init.client_id,
            &["AZURE_SENTINEL_CLIENT_ID", "AZURE_CLIENT_ID"],
        )
        .ok_or(SentinelError::MissingClientId)?,
        client_secret: first_set(
            init.client_secret.as_ref().map(ExposeSecret::expose_secret),
            environment,
            &["AZURE_SENTINEL_CLIENT_SECRET", "AZURE_CLIENT_SECRET"],
        )
        .ok_or(SentinelError::MissingClientSecret)?,
    })
}

impl SentinelSettings {
    pub fn resolve(
        init: &SentinelInit,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, SentinelError> {
        let first = |argument: &Option<String>, names: &[&str]| {
            first_set(argument.as_deref(), environment, names)
        };
        let stream_name = first(&init.stream_name, &["AZURE_SENTINEL_STREAM_NAME"])
            .unwrap_or_else(|| DEFAULT_STREAM_NAME.to_owned());
        let audit_stream_name = first(
            &init.audit_stream_name,
            &["AZURE_SENTINEL_AUDIT_STREAM_NAME"],
        )
        .unwrap_or_else(|| stream_name.clone());
        let authority_host = normalize_authority_host(
            &first(
                &init.authority_host,
                &["AZURE_SENTINEL_AUTHORITY_HOST", "AZURE_AUTHORITY_HOST"],
            )
            .unwrap_or_else(|| DEFAULT_AUTHORITY_HOST.to_owned()),
        );
        let found = credentials(init, environment)?;
        let batch_variables: BTreeMap<String, String> =
            [BATCH_SIZE_VARIABLE, FLUSH_INTERVAL_VARIABLE]
                .into_iter()
                .filter_map(|name| Some((name.to_owned(), environment.lookup(name)?)))
                .collect();
        Ok(Self {
            model_url: api_endpoint(&found.endpoint, &found.dcr_immutable_id, &stream_name),
            audit_url: api_endpoint(&found.endpoint, &found.dcr_immutable_id, &audit_stream_name),
            token_url: format!("{authority_host}/{}/oauth2/v2.0/token", found.tenant_id),
            scope: resolve_oauth_scope(&authority_host),
            client_id: found.client_id,
            client_secret: SecretString::from(found.client_secret),
            batch: BatchSettings::from_environment(&batch_variables)?,
        })
    }
}

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::sections::azure_url_policy::{AzureCredentials, Settings};
use betterlite_config::types::Config;
use secrecy::SecretString;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use crate::credentials::selection::AzureTokenError;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::oidc_exchange::OidcExchangeInput;

const BASE: &str =
    "model_list:\n  - model_name: azure-urls\n    litellm_params:\n      model: azure/gpt-4o\n";
const TOKEN_BASE: &str =
    "model_list:\n  - model_name: azure-token\n    litellm_params:\n      model: azure/gpt-4o\n";

pub(super) fn environment_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(source: &str, environment: &[(&str, &str)]) -> Config {
    ConfigLoader {
        lookup: environment_map(environment),
    }
    .load(source)
    .unwrap()
}

pub(super) fn load(extra: &str, environment: &[(&str, &str)]) -> Settings {
    config(&format!("{BASE}{extra}"), environment)
        .sections
        .azure_url_policy
}

pub(super) fn credentials(extra: &str, environment: &[(&str, &str)]) -> AzureCredentials {
    load(extra, environment)
        .deployments
        .first()
        .unwrap()
        .clone()
}

pub(super) fn token_settings(extra: &str, environment: &[(&str, &str)]) -> DeploymentSettings {
    let source = format!("{TOKEN_BASE}{extra}");
    config(&source, environment)
        .sections
        .azure_credentials
        .deployments
        .first()
        .unwrap()
        .clone()
}

pub(super) fn environment_headers(
    headers: &[(&str, &str)],
    api_key: Option<&SecretString>,
    credentials: &DeploymentSettings,
) -> Result<BTreeMap<String, String>, AzureTokenError> {
    let sources = NoSources;
    validate_azure_environment(EnvironmentHeadersRequest {
        headers: environment_map(headers),
        api_key,
        credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
}

pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

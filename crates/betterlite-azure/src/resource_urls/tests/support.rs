use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use secrecy::SecretString;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::oidc_exchange::OidcExchangeInput;
use crate::resource_urls::ResourceAuth;

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

pub(super) fn settings(azure_ad_token: Option<&str>) -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: azure_ad_token.map(SecretString::from),
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: DEFAULT_SCOPE.to_owned(),
        azure_username: None,
        azure_password: None,
    }
}

pub(super) fn header_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn auth<'data>(
    headers: &[(&str, &str)],
    fallback_api_key: Option<&'data SecretString>,
    credentials: &'data DeploymentSettings,
) -> ResourceAuth<'data> {
    ResourceAuth {
        headers: header_map(headers),
        fallback_api_key,
        credentials,
        refresh_enabled: false,
        provider: None,
        sources: &NoSources,
    }
}

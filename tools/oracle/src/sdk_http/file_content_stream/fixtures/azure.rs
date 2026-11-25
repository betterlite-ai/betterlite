use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::azure_files::client::{
    ClientMaterializer, ClientSettings, Connection, Opened, SelectedClient,
};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, ClientOutcome, SessionKind,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_core::api::file_content_stream::target::AzureContent;
use secrecy::SecretString;

// The client the azure files handler would build for this call: its api_base, api_key and api_version
pub(crate) fn open(
    content: &AzureContent,
    environment: &BTreeMap<String, String>,
) -> Result<Opened, AzureFilesError> {
    let endpoints = EndpointSettings::default();
    let sources = Sources;
    let materializer = Materializer;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    Connection {
        settings: ClientSettings {
            api_key: content.api_key.clone(),
            api_base: content.api_base.clone(),
            api_version: content.api_version.clone(),
        },
        params: content.params.clone(),
        provider: None,
        supplied: None,
        is_async: content.mode.is_async,
        endpoints: &endpoints,
        environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    }
    .open()
}

// the case hands no credential source and no cached client, so every path that would read one refuses
pub(crate) struct Sources;

impl CredentialSources for Sources {
    fn client_secret(&self, _record: &ClientSecretRecord) -> Failure<'_> {
        Err(unavailable())
    }

    fn username_password(&self, _record: &UsernamePasswordRecord) -> Failure<'_> {
        Err(unavailable())
    }

    fn sdk_refresh(&self, _scope: &str) -> Failure<'_> {
        Err(unavailable())
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable())
    }
}

type Failure<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

fn unavailable() -> SourceFailure {
    SourceFailure {
        class: "RuntimeError".to_owned(),
        status: None,
        message: "the case carries no credential source".to_owned(),
    }
}

pub(crate) struct Cache;

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

// nothing is supplied or cached, so the handler always builds the client from its own arguments
pub(crate) struct Materializer;

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        _outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError> {
        Ok(None)
    }
}

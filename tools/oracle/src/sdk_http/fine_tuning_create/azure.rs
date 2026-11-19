use super::client::{Auth, HttpJobsApi};
use super::native::{SendError, seconds};
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    AzureClientInputs, ClientIdentity, ClientInputs, ClientKind, ClientOutcome,
    OpenAiV1ClientInputs, SessionKind, V1Key,
};
use betterlite_azure::fine_tuning::client::{ClientMaterializer, Connection};
use betterlite_azure::fine_tuning::create_fine_tuning_job;
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_core::api::fine_tuning_create::AzureCreateCall;
use betterlite_openai::fine_tuning::client::{BuildError, ClientSettings, JobsApi};
use betterlite_openai::fine_tuning::job::FineTuneJob;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn create(call: AzureCreateCall) -> Result<FineTuneJob, SendError> {
    let AzureCreateCall {
        is_async,
        mut data,
        api_base,
        api_key,
        api_version,
        organization,
        timeout,
        max_retries,
    } = call;
    let settings = ClientSettings {
        api_key,
        api_base,
        api_version,
        timeout: seconds(timeout),
        max_retries,
        organization,
    };
    let endpoints = EndpointSettings::default();
    let environment = BTreeMap::new();
    let sources = Sources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let connection = Connection {
        settings,
        client: None,
        is_async,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &Materializer,
    };
    create_fine_tuning_job(connection, &mut data).map_err(SendError::from)
}
struct Materializer;
impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        let Some(inputs) = &outcome.inputs else {
            return Ok(None);
        };
        let is_async = matches!(
            outcome.kind,
            ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi
        );
        Ok(match inputs {
            ClientInputs::Azure(azure) => azure_api(azure, is_async),
            ClientInputs::OpenAiV1(v1) => v1_api(v1, is_async),
        })
    }
}
// AzureOpenAI builds "{azure_endpoint}/openai" and appends the api-version query
fn azure_api(azure: &AzureClientInputs<'_>, is_async: bool) -> Option<Box<dyn JobsApi>> {
    let base = match (&azure.endpoint.base_url, &azure.endpoint.azure_endpoint) {
        (Some(base), _) => base.trim_end_matches('/').to_owned(),
        (None, Some(endpoint)) => format!("{}/openai", endpoint.trim_end_matches('/')),
        (None, None) => return None,
    };
    Some(Box::new(HttpJobsApi {
        is_async,
        url: format!("{base}/fine_tuning/jobs?api-version={}", azure.api_version),
        api_key: azure.api_key.clone(),
        organization: None,
        auth: Auth::ApiKey,
        timeout: azure.timeout,
    }))
}
// a v1 api-version builds the standard OpenAI client against "{api_base}/openai/v1/"
fn v1_api(v1: &OpenAiV1ClientInputs<'_>, is_async: bool) -> Option<Box<dyn JobsApi>> {
    let base = v1.base_url.as_deref()?.trim_end_matches('/').to_owned();
    let api_key = match &v1.key {
        Some(V1Key::Static(key)) => Some(key.clone()),
        Some(V1Key::Provider { .. }) | None => None,
    };
    Some(Box::new(HttpJobsApi {
        is_async,
        url: format!("{base}/fine_tuning/jobs"),
        api_key,
        organization: None,
        auth: Auth::Bearer,
        timeout: v1.timeout,
    }))
}
struct Sources;
fn source_failure() -> SourceFailure {
    SourceFailure {
        class: "Unavailable".to_owned(),
        status: None,
        message: "the oracle has no credential sources".to_owned(),
    }
}
impl CredentialSources for Sources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }
    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }
    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }
    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }
    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(source_failure())
    }
}
#[derive(Default)]
struct Cache;
impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }
    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

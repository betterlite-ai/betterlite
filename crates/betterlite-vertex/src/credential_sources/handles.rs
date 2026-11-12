use std::path::Path;
use std::sync::Arc;

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialSource, decode_vertex_credentials,
};
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;

use super::adc::{AdcDeps, credentials_from_default_auth};
use super::aws::AwsCredentialsSource;
use super::error::CredentialSourceError;
use super::files::FileReader;
use super::sdk::{CredentialBuilders, CredentialClock, TokenSource};
use crate::aws_federation::FederationInput;

pub const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

pub struct LoadDeps<'deps> {
    pub environment: &'deps AdcEnvironment,
    pub files: &'deps dyn FileReader,
    pub http: &'deps dyn CredentialBuilders,
    pub aws: Arc<dyn AwsCredentialsSource>,
    pub clock: Arc<dyn CredentialClock>,
}

pub struct LoadedAuth {
    pub handle: Arc<dyn TokenSource>,
    pub project: String,
}

// the credential handle carries secrets, so only the resolved project reaches a diagnostic
impl std::fmt::Debug for LoadedAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedAuth")
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

// VertexBase.load_auth: build the credential from the configured source or ADC, refresh it, then check the project
pub async fn load_auth(
    credentials: Option<&ConfiguredCredentials>,
    project: Option<&str>,
    deps: &LoadDeps<'_>,
) -> Result<LoadedAuth, CredentialSourceError> {
    let (handle, recovered) = match credentials {
        Some(configured) => {
            let source = decode_configured(configured, deps.files)?;
            let handle = build_configured(&source, deps)?;
            let recovered = source
                .project_id()
                .or_else(|| source.quota_project_id())
                .map(str::to_owned);
            (handle, recovered)
        }
        None => {
            let adc = AdcDeps {
                environment: deps.environment,
                files: deps.files,
                http: deps.http,
            };
            credentials_from_default_auth(&[CLOUD_PLATFORM_SCOPE.to_owned()], &adc).await?
        }
    };
    // LiteLLM refreshes the new handle before it checks whether a project resolved
    handle.access_token().await?;
    let project = project
        .map(str::to_owned)
        .or(recovered)
        .filter(|text| !text.is_empty())
        .ok_or(CredentialSourceError::MissingProject)?;
    Ok(LoadedAuth { handle, project })
}

// LiteLLM's credentials value is a JSON object, a JSON string or an existing local filename
pub(crate) fn decode_configured(
    configured: &ConfiguredCredentials,
    files: &dyn FileReader,
) -> Result<CredentialSource, CredentialSourceError> {
    match configured {
        ConfiguredCredentials::Source(source) => Ok(source.clone()),
        ConfiguredCredentials::Text(text) => {
            let decoded = match files.read(Path::new(text)) {
                Ok(bytes) => decode_vertex_credentials(&bytes),
                Err(_source) => decode_vertex_credentials(text.as_bytes()),
            };
            decoded.map_err(|source| CredentialSourceError::Credentials { source })
        }
    }
}

fn build_configured(
    source: &CredentialSource,
    deps: &LoadDeps<'_>,
) -> Result<Arc<dyn TokenSource>, CredentialSourceError> {
    let scopes = [CLOUD_PLATFORM_SCOPE.to_owned()];
    match source {
        CredentialSource::ServiceAccount(account) => {
            deps.http.service_account(&account.document(), &scopes)
        }
        CredentialSource::AuthorizedUser(user) => {
            deps.http.authorized_user(&user.document(), &scopes)
        }
        CredentialSource::ExternalAccount(account) => {
            let document = account.document();
            let Some(environment_id) = account.aws_environment() else {
                if account.is_executable() {
                    return deps.http.pluggable(&document, &scopes);
                }
                return deps.http.identity_pool(&document, &scopes);
            };
            if !environment_id.contains("aws") {
                return Err(CredentialSourceError::InvalidIdentityPoolEnvironment);
            }
            if account.aws.any() {
                let input = serde_json::from_str::<FederationInput>(&document)
                    .map_err(CredentialSourceError::Document)?;
                return deps.http.aws_federation(
                    input,
                    Arc::clone(&deps.aws),
                    Arc::clone(&deps.clock),
                );
            }
            deps.http.identity_pool_with_aws(&document, &scopes)
        }
    }
}

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::{Credentials, SourceError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_vertex::aws_federation::AwsCredentialsResolver;
use betterlite_vertex::credential_sources::aws::AwsCredentialsSource;
use betterlite_vertex::credential_sources::files::StdFiles;
use betterlite_vertex::credential_sources::handles::{LoadDeps, load_auth};
use betterlite_vertex::credential_sources::sdk::{CredentialClock, GoogleAuth, TokenSource};
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use secrecy::{ExposeSecret as _, SecretString};

// the SDK handle keeps its own token and reports no expiry, so a short assumed lifetime only makes the cache ask the handle again
const ASSUMED_TOKEN_LIFETIME: Duration = Duration::from_secs(600);

// the server has no AWS credential provider for a Vertex workload identity pool, so that source fails the load
pub(crate) struct UnavailableAws;

impl AwsCredentialsResolver for UnavailableAws {
    async fn credentials(&self, _settings: &AwsSettings) -> Result<Credentials, CredentialError> {
        Err(CredentialError::Source(SourceError::MissingProvider))
    }
}

// VertexBase.load_auth and refresh_auth over the SDK handles, each kept under the token it last produced
pub(crate) struct VertexCredentialExecutor {
    pub(crate) environment: AdcEnvironment,
    pub(crate) google: GoogleAuth,
    pub(crate) clock: Arc<dyn CredentialClock>,
    pub(crate) handles: Mutex<BTreeMap<String, Arc<dyn TokenSource>>>,
}

impl VertexCredentialExecutor {
    fn track(
        &self,
        stale: Option<&str>,
        handle: Arc<dyn TokenSource>,
        token: SecretString,
        project: Option<String>,
    ) -> CredentialSnapshot {
        let mut handles = self.handles.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = stale {
            handles.remove(previous);
        }
        handles.insert(token.expose_secret().to_owned(), handle);
        CredentialSnapshot {
            token: Some(token),
            state: None,
            expires_at: self.clock.now().checked_add(ASSUMED_TOKEN_LIFETIME),
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: project,
        }
    }
}

impl CredentialExecutor for VertexCredentialExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async move {
            let files = StdFiles;
            let deps = LoadDeps {
                environment: &self.environment,
                files: &files,
                http: &self.google,
                aws: Arc::new(UnavailableAws),
                clock: Arc::clone(&self.clock),
            };
            let loaded = load_auth(credentials, project, &deps)
                .await
                .map_err(CredentialCacheError::Load)?;
            let token = loaded
                .handle
                .access_token()
                .await
                .map_err(CredentialCacheError::Load)?;
            let snapshot = self.track(None, loaded.handle, token, Some(loaded.project.clone()));
            Ok(LoadedCredential {
                credential: Some(Arc::new(snapshot)),
                project: Some(loaded.project),
            })
        })
    }

    fn refresh<'borrow>(
        &'borrow self,
        credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async move {
            let stale = credential
                .token
                .as_ref()
                .ok_or(CredentialCacheError::MissingToken)?
                .expose_secret();
            let found = self
                .handles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(stale)
                .cloned();
            let handle = found.ok_or(CredentialCacheError::MissingCredentials)?;
            let token = handle
                .access_token()
                .await
                .map_err(CredentialCacheError::Refresh)?;
            Ok(self.track(Some(stale), handle, token, credential.project_id.clone()))
        })
    }
}

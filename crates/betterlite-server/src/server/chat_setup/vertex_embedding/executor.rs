use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_bedrock::aws_settings::sources::{Credentials, SourceError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_vertex::aws_federation::AwsCredentialsResolver;
use betterlite_vertex::credential_sources::files::StdFiles;
use betterlite_vertex::credential_sources::handles::{LoadDeps, load_auth};
use betterlite_vertex::credential_sources::sdk::{CredentialClock, GoogleAuth, TokenSource};
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

type Pending<'borrow, Value> =
    Pin<Box<dyn Future<Output = Result<Value, CredentialCacheError>> + Send + 'borrow>>;

// the SDK handle keeps its own token and reports no expiry, so a short assumed lifetime only makes the cache ask the handle again
const ASSUMED_TOKEN_LIFETIME: Duration = Duration::from_secs(600);

// the server has no AWS credential provider for a Vertex workload identity pool, so that source fails the load
pub(crate) struct UnavailableAws;

impl AwsCredentialsResolver for UnavailableAws {
    fn credentials(
        &self,
        _settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Err(CredentialError::Source(SourceError::MissingProvider)))
    }
}

// VertexBase.load_auth and refresh_auth over the SDK handles, each kept under the SHA-256 of the token it last produced so no plain copy of a token outlives its SecretString
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
        handles.insert(
            hex::encode(Sha256::digest(token.expose_secret().as_bytes())),
            handle,
        );
        drop(handles);
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
    ) -> Pending<'borrow, LoadedCredential> {
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
    ) -> Pending<'borrow, CredentialSnapshot> {
        Box::pin(async move {
            let token = credential
                .token
                .as_ref()
                .ok_or(CredentialCacheError::MissingToken)?;
            let stale = hex::encode(Sha256::digest(token.expose_secret().as_bytes()));
            let found = self
                .handles
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&stale)
                .cloned();
            let handle = found.ok_or(CredentialCacheError::MissingCredentials)?;
            let refreshed = handle
                .access_token()
                .await
                .map_err(CredentialCacheError::Refresh)?;
            Ok(self.track(
                Some(&stale),
                handle,
                refreshed,
                credential.project_id.clone(),
            ))
        })
    }
}

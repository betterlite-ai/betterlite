use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialDecodeError, CredentialSource,
};
use google_cloud_auth::build_errors::Error as BuildError;
use google_cloud_auth::credentials::service_account::{
    AccessSpecifier, Builder as ServiceAccountBuilder,
};
use google_cloud_auth::credentials::user_account::Builder as UserAccountBuilder;
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use google_cloud_auth::errors::CredentialsError;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::gcs_client::gcs_storage_auth::STORAGE_ENDPOINT;
use crate::gcs_client::gcs_storage_auth::caching_key_behaviors::{
    CredentialKey, vertex_instance_key,
};
use crate::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GetTokenAndUrl, TokenAndUrl, VertexAuth,
};

// LiteLLM asks google-auth for the cloud-platform scope, the one that reaches Cloud Storage
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("the vertex credentials file could not be read")]
    Read(#[source] io::Error),
    #[error("the vertex credentials could not be read by the Google auth SDK")]
    Parse(#[source] serde_json::Error),
    #[error("the vertex credentials are not a credential document")]
    Document,
    #[error("the vertex credentials could not be decoded")]
    Decode(#[source] CredentialDecodeError),
    #[error("vertex credentials of type {kind} are not supported by the GCS logging backend")]
    Unsupported { kind: String },
    #[error("the vertex credentials could not be built")]
    Build(#[source] BuildError),
    #[error("the access token request failed")]
    Token(#[source] CredentialsError),
    #[error("the GCS request carries no access token to return")]
    MissingToken,
    #[error("the synchronous GCS token path is not supported; the logging path is asynchronous")]
    Synchronous,
}

// one built credential per configured document, since the crate caches the token inside it
#[derive(Clone, Debug, Default)]
pub struct GoogleCloudAuth {
    credentials: Arc<Mutex<BTreeMap<CredentialKey, AccessTokenCredentials>>>,
}

impl GoogleCloudAuth {
    fn credentials_for(
        &self,
        key: &CredentialKey,
        supplied: Option<&SecretString>,
    ) -> Result<AccessTokenCredentials, AuthError> {
        let cached = self
            .credentials
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned();
        if let Some(credentials) = cached {
            return Ok(credentials);
        }
        let built = build_credentials(supplied)?;
        self.credentials
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), built.clone());
        Ok(built)
    }
}

fn build_credentials(supplied: Option<&SecretString>) -> Result<AccessTokenCredentials, AuthError> {
    let Some(secret) = supplied else {
        return Builder::default()
            .with_scopes([CLOUD_PLATFORM_SCOPE])
            .build_access_token_credentials()
            .map_err(AuthError::Build);
    };
    let text = credential_text(secret.expose_secret())?;
    let ConfiguredCredentials::Source(source) =
        ConfiguredCredentials::from_text(text).map_err(AuthError::Decode)?
    else {
        return Err(AuthError::Document);
    };
    match &source {
        CredentialSource::ServiceAccount(_) => {
            let key = serde_json::from_str(&source.document()).map_err(AuthError::Parse)?;
            ServiceAccountBuilder::new(key)
                .with_access_specifier(AccessSpecifier::from_scopes([CLOUD_PLATFORM_SCOPE]))
                .build_access_token_credentials()
                .map_err(AuthError::Build)
        }
        CredentialSource::AuthorizedUser(_) => {
            let key = serde_json::from_str(&source.document()).map_err(AuthError::Parse)?;
            UserAccountBuilder::new(key)
                .with_scopes([CLOUD_PLATFORM_SCOPE])
                .build_access_token_credentials()
                .map_err(AuthError::Build)
        }
        CredentialSource::ExternalAccount(_) => Err(AuthError::Unsupported {
            kind: "external_account".to_owned(),
        }),
        CredentialSource::Impersonated(_) => Err(AuthError::Unsupported {
            kind: "impersonated_service_account".to_owned(),
        }),
    }
}

// GCS_PATH_SERVICE_ACCOUNT holds a path to a service account file or the document itself
fn credential_text(raw: &str) -> Result<String, AuthError> {
    if Path::new(raw).is_file() {
        return fs::read_to_string(raw).map_err(AuthError::Read);
    }
    Ok(raw.to_owned())
}

#[async_trait]
impl VertexAuth for GoogleCloudAuth {
    type Error = AuthError;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, AuthError> {
        let key = CredentialKey(vertex_instance_key(request.credentials.as_ref()));
        let credentials = self.credentials_for(&key, request.credentials.as_ref())?;
        let token = credentials.access_token().await.map_err(AuthError::Token)?;
        Ok(AccessToken {
            token: SecretString::from(token.token),
            project_id: request.project_id.unwrap_or_default(),
        })
    }

    fn ensure_access_token(&self, _request: EnsureAccessToken) -> Result<AccessToken, AuthError> {
        Err(AuthError::Synchronous)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, AuthError> {
        let token = request.auth_header.ok_or(AuthError::MissingToken)?;
        Ok(TokenAndUrl {
            token,
            url: STORAGE_ENDPOINT.to_owned(),
        })
    }
}

use std::error::Error;

use async_trait::async_trait;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;

use super::installation::Installation;

pub const MOCK_TOKEN: &str = "mock-gcs-token";
pub const MOCK_PROJECT_ID: &str = "mock-project-id";
pub const MOCK_STORAGE_URL: &str = "https://storage.googleapis.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexAuthProvider {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl From<VertexAuthProvider> for ProviderId {
    fn from(provider: VertexAuthProvider) -> Self {
        match provider {
            VertexAuthProvider::VertexAi => Self::VertexAi,
            VertexAuthProvider::VertexAiBeta => Self::VertexAiBeta,
            VertexAuthProvider::Gemini => Self::Gemini,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnsureAccessToken {
    pub credentials: Option<SecretString>,
    pub project_id: Option<String>,
    pub custom_llm_provider: VertexAuthProvider,
}

#[derive(Debug, Clone)]
pub struct GetTokenAndUrl {
    pub model: String,
    pub auth_header: Option<SecretString>,
    pub vertex_credentials: Option<SecretString>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub gemini_api_key: Option<SecretString>,
    pub stream: Option<bool>,
    pub custom_llm_provider: VertexAuthProvider,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: SecretString,
    pub project_id: String,
}

#[derive(Debug, Clone)]
pub struct TokenAndUrl {
    pub token: SecretString,
    pub url: String,
}

#[async_trait]
pub trait VertexAuth: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, Self::Error>;

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, Self::Error>;

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, Self::Error>;
}

pub struct GcsMockAuth<Real> {
    pub real: Real,
    pub installed: bool,
}

impl<Real> GcsMockAuth<Real> {
    pub const fn install(&mut self) -> Installation {
        if self.installed {
            return Installation::AlreadyInstalled;
        }
        self.installed = true;
        Installation::Installed
    }
}

#[async_trait]
impl<Real> VertexAuth for GcsMockAuth<Real>
where
    Real: VertexAuth,
{
    type Error = Real::Error;

    async fn ensure_access_token_async(
        &self,
        request: EnsureAccessToken,
    ) -> Result<AccessToken, Self::Error> {
        if let Some(token) = mock_access_token(self.installed) {
            return Ok(token);
        }
        self.real.ensure_access_token_async(request).await
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, Self::Error> {
        if let Some(token) = mock_access_token(self.installed) {
            return Ok(token);
        }
        self.real.ensure_access_token(request)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, Self::Error> {
        if let Some(token) = mock_token_and_url(self.installed) {
            return Ok(token);
        }
        self.real.get_token_and_url(request)
    }
}

fn mock_access_token(installed: bool) -> Option<AccessToken> {
    installed.then(|| AccessToken {
        token: SecretString::from(MOCK_TOKEN),
        project_id: MOCK_PROJECT_ID.to_owned(),
    })
}

fn mock_token_and_url(installed: bool) -> Option<TokenAndUrl> {
    let access = mock_access_token(installed)?;
    Some(TokenAndUrl {
        token: access.token,
        url: MOCK_STORAGE_URL.to_owned(),
    })
}

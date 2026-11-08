use std::error::Error;

use async_trait::async_trait;

use super::installation::Installation;

pub const MOCK_TOKEN: &str = "mock-gcs-token";
pub const MOCK_PROJECT_ID: &str = "mock-project-id";
pub const MOCK_STORAGE_URL: &str = "https://storage.googleapis.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureAccessToken {
    pub credentials: Option<String>,
    pub project_id: Option<String>,
    pub custom_llm_provider: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct GetTokenAndUrl {
    pub model: String,
    pub auth_header: Option<String>,
    pub vertex_credentials: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub gemini_api_key: Option<String>,
    pub stream: Option<bool>,
    pub custom_llm_provider: String,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessToken {
    pub token: String,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAndUrl {
    pub token: String,
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

pub const fn mock_vertex_auth_methods<Real>(real: Real) -> GcsMockAuth<Real>
where
    Real: VertexAuth,
{
    let mut auth = GcsMockAuth {
        real,
        installed: false,
    };
    let _installation = auth.install();
    auth
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
        if self.installed {
            return Ok(AccessToken {
                token: MOCK_TOKEN.to_owned(),
                project_id: MOCK_PROJECT_ID.to_owned(),
            });
        }
        self.real.ensure_access_token_async(request).await
    }

    fn ensure_access_token(&self, request: EnsureAccessToken) -> Result<AccessToken, Self::Error> {
        if self.installed {
            return Ok(AccessToken {
                token: MOCK_TOKEN.to_owned(),
                project_id: MOCK_PROJECT_ID.to_owned(),
            });
        }
        self.real.ensure_access_token(request)
    }

    fn get_token_and_url(&self, request: GetTokenAndUrl) -> Result<TokenAndUrl, Self::Error> {
        if self.installed {
            return Ok(TokenAndUrl {
                token: MOCK_TOKEN.to_owned(),
                url: MOCK_STORAGE_URL.to_owned(),
            });
        }
        self.real.get_token_and_url(request)
    }
}

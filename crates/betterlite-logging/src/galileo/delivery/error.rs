use super::transport::TransportError;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("username, password or base URL is not set")]
    NotConfigured,
    #[error("login request failed: {0}")]
    Login(#[from] TransportError),
    #[error("login returned HTTP {status}")]
    LoginStatus { status: u16 },
    #[error("login response carries no access_token")]
    LoginBody(#[source] serde_json::Error),
    #[error("GALILEO_PROJECT_ID environment variable not set")]
    MissingProjectId,
    #[error("GALILEO_BASE_URL environment variable not set")]
    MissingBaseUrl,
    #[error(
        "GALILEO_API_KEY or GALILEO_USERNAME and GALILEO_PASSWORD environment variables must be set"
    )]
    MissingCredentials,
    #[error("Galileo authentication failed")]
    AuthenticationFailed,
    #[error("Galileo API returned HTTP {status}")]
    ProbeStatus { status: u16 },
    #[error("Galileo health check failed: {0}")]
    ProbeFailed(#[source] TransportError),
}

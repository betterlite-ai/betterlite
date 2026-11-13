use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

const DEFAULT_ENDPOINT: &str = "https://otlp.arize.com/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlpProtocol {
    Grpc,
    Http,
}

impl OtlpProtocol {
    #[must_use]
    pub const fn exporter_kind(self) -> &'static str {
        match self {
            Self::Grpc => "otlp_grpc",
            Self::Http => "otlp_http",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArizeConfig {
    pub space_id: Option<String>,
    pub space_key: Option<String>,
    pub api_key: Option<SecretString>,
    pub protocol: OtlpProtocol,
    pub endpoint: String,
    pub project_name: Option<String>,
}

impl ArizeConfig {
    #[must_use]
    pub fn from_env(environment: &dyn EnvironmentLookup) -> Self {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let (protocol, endpoint) = match (read("ARIZE_ENDPOINT"), read("ARIZE_HTTP_ENDPOINT")) {
            (Some(endpoint), _) => (OtlpProtocol::Grpc, endpoint),
            (None, Some(endpoint)) => (OtlpProtocol::Http, endpoint),
            (None, None) => (OtlpProtocol::Grpc, DEFAULT_ENDPOINT.to_owned()),
        };
        Self {
            space_id: read("ARIZE_SPACE_ID"),
            space_key: read("ARIZE_SPACE_KEY"),
            api_key: read("ARIZE_API_KEY").map(SecretString::from),
            protocol,
            endpoint,
            project_name: read("ARIZE_PROJECT_NAME"),
        }
    }
}

mod tests;

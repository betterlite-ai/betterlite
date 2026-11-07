use betterlite_config::loader::EnvironmentLookup;

use super::arize_settings::OtlpProtocol;

const DEFAULT_ENDPOINT: &str = "http://localhost:6006/v1/traces";
const PHOENIX_CLOUD_HOST: &str = "app.phoenix.arize.com";
const TRACES_PATH: &str = "/v1/traces";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoenixConfig {
    pub otlp_auth_headers: Option<String>,
    pub protocol: OtlpProtocol,
    pub endpoint: String,
    pub project_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PhoenixSettingsError {
    #[error("PHOENIX_API_KEY must be set when using Phoenix Cloud (app.phoenix.arize.com).")]
    CloudApiKeyMissing,
}

impl PhoenixConfig {
    pub fn from_env(environment: &dyn EnvironmentLookup) -> Result<Self, PhoenixSettingsError> {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let collector_endpoint =
            read("PHOENIX_COLLECTOR_HTTP_ENDPOINT").or_else(|| read("PHOENIX_COLLECTOR_ENDPOINT"));
        let (protocol, endpoint) = match collector_endpoint {
            None => (OtlpProtocol::Http, DEFAULT_ENDPOINT.to_owned()),
            Some(endpoint) if is_grpc_endpoint(&endpoint) => (OtlpProtocol::Grpc, endpoint),
            Some(endpoint) => (OtlpProtocol::Http, http_endpoint(endpoint)),
        };
        let otlp_auth_headers = match environment.lookup("PHOENIX_API_KEY") {
            Some(api_key) => {
                let header = match protocol {
                    OtlpProtocol::Grpc => "authorization",
                    OtlpProtocol::Http => "Authorization",
                };
                Some(format!("{header}=Bearer {api_key}"))
            }
            None if endpoint.contains(PHOENIX_CLOUD_HOST) => {
                return Err(PhoenixSettingsError::CloudApiKeyMissing);
            }
            None => None,
        };
        Ok(Self {
            otlp_auth_headers,
            protocol,
            endpoint,
            project_name: read("PHOENIX_PROJECT_NAME").unwrap_or_else(|| "default".to_owned()),
        })
    }
}

fn is_grpc_endpoint(endpoint: &str) -> bool {
    let grpc_scheme = endpoint.starts_with("grpc://");
    let grpc_port = endpoint.contains(":4317") && !endpoint.contains(TRACES_PATH);
    grpc_scheme || grpc_port
}

fn http_endpoint(endpoint: String) -> String {
    if endpoint.contains(PHOENIX_CLOUD_HOST) || endpoint.contains(TRACES_PATH) {
        endpoint
    } else if endpoint.ends_with("/v1") {
        format!("{endpoint}/traces")
    } else if endpoint.ends_with('/') {
        format!("{endpoint}v1/traces")
    } else {
        format!("{endpoint}{TRACES_PATH}")
    }
}

mod tests;

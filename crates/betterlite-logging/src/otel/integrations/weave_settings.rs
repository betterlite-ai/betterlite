use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::arize_settings::OtlpProtocol;

pub const WEAVE_BASE_URL: &str = "https://trace.wandb.ai";
pub const WEAVE_OTEL_ENDPOINT: &str = "/otel/v1/traces";

#[derive(Debug, Clone)]
pub struct WeaveOtelConfig {
    pub otlp_auth_headers: SecretString,
    pub endpoint: String,
    pub project_id: String,
    pub protocol: OtlpProtocol,
}

#[derive(Debug, thiserror::Error)]
pub enum WeaveSettingsError {
    #[error("WANDB_API_KEY must be set for Weave OpenTelemetry integration.")]
    ApiKey,
    #[error(
        "WANDB_PROJECT_ID must be set for Weave OpenTelemetry integration. Format: <entity>/<project_name>"
    )]
    ProjectId,
}

impl WeaveOtelConfig {
    pub fn from_env(environment: &dyn EnvironmentLookup) -> Result<Self, WeaveSettingsError> {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        let api_key = SecretString::from(read("WANDB_API_KEY").ok_or(WeaveSettingsError::ApiKey)?);
        let project_id = read("WANDB_PROJECT_ID").ok_or(WeaveSettingsError::ProjectId)?;
        let endpoint = weave_otel_endpoint(read("WANDB_HOST").as_deref());
        Ok(Self {
            otlp_auth_headers: SecretString::from(format!(
                "Authorization={},project_id={project_id}",
                weave_authorization_header(api_key.expose_secret())
            )),
            endpoint,
            project_id,
            protocol: OtlpProtocol::Http,
        })
    }
}

#[must_use]
pub fn weave_authorization_header(api_key: &str) -> String {
    let encoded = STANDARD.encode(format!("api:{api_key}"));
    format!("Basic {encoded}")
}

#[must_use]
pub fn weave_otel_endpoint(host: Option<&str>) -> String {
    match host {
        None | Some("") => format!("{WEAVE_BASE_URL}{WEAVE_OTEL_ENDPOINT}"),
        Some(value) => {
            let base = if value.starts_with("http") {
                value.to_owned()
            } else {
                format!("https://{value}")
            };
            format!("{}{WEAVE_OTEL_ENDPOINT}", base.trim_end_matches('/'))
        }
    }
}

mod tests;

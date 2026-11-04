use std::collections::BTreeMap;
use std::path::Path;

use crate::types::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    Enabled,
    Disabled,
    CaFile(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsPolicy {
    pub verification: Verification,
    pub ca_file: Option<String>,
    pub certificate: Option<String>,
    pub http2: bool,
    pub ecdh_curve: Option<String>,
    pub security_level: Option<String>,
    pub user_agent: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TlsPolicyError {
    #[error("SSL_SECURITY_LEVEL is not supported by rustls")]
    SecurityLevel,
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

impl TlsPolicy {
    pub fn resolve(
        config: &Config,
        environment: &BTreeMap<String, String>,
        deployment_ssl_verify: Option<bool>,
    ) -> Result<Self, TlsPolicyError> {
        let verification = resolve_verification(config, environment, deployment_ssl_verify)?;
        let security_level = config
            .litellm_settings
            .ssl_security_level
            .clone()
            .or_else(|| environment.get("SSL_SECURITY_LEVEL").cloned());
        if security_level.is_some() {
            return Err(TlsPolicyError::SecurityLevel);
        }
        let verification = match (
            verification,
            environment
                .get("SSL_CERT_FILE")
                .filter(|path| Path::new(path).exists()),
        ) {
            (Verification::Enabled, Some(path)) => Verification::CaFile(path.clone()),
            (selected, _) => selected,
        };
        Ok(Self {
            verification,
            ca_file: environment.get("SSL_CERT_FILE").cloned(),
            certificate: config
                .litellm_settings
                .ssl_certificate
                .clone()
                .or_else(|| environment.get("SSL_CERTIFICATE").cloned()),
            http2: config.litellm_settings.http2 == Some(true)
                || betterlite_env::flag(
                    "LITELLM_HTTP2",
                    environment.get("LITELLM_HTTP2").map_or("", String::as_str),
                )?
                .unwrap_or(false),
            ecdh_curve: config
                .litellm_settings
                .ssl_ecdh_curve
                .clone()
                .or_else(|| environment.get("SSL_ECDH_CURVE").cloned()),
            security_level,
            user_agent: environment
                .get("LITELLM_USER_AGENT")
                .cloned()
                .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION"))),
        })
    }
}

fn resolve_verification(
    config: &Config,
    environment: &BTreeMap<String, String>,
    deployment_ssl_verify: Option<bool>,
) -> Result<Verification, TlsPolicyError> {
    let value = if let Some(value) = deployment_ssl_verify {
        Some(value)
    } else {
        let environment_value = betterlite_env::flag(
            "SSL_VERIFY",
            environment.get("SSL_VERIFY").map_or("", String::as_str),
        )?;
        environment_value.or(config.litellm_settings.ssl_verify)
    };
    Ok(match value {
        Some(true) | None => Verification::Enabled,
        Some(false) => Verification::Disabled,
    })
}

mod tests;

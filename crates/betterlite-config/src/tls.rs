use std::collections::BTreeMap;

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
        let verification = if let Some(value) = deployment_ssl_verify {
            if value {
                Verification::Enabled
            } else {
                Verification::Disabled
            }
        } else if let Some(value) = config.litellm_settings.ssl_verify {
            if value {
                Verification::Enabled
            } else {
                Verification::Disabled
            }
        } else if let Some(value) = betterlite_env::flag(
            "SSL_VERIFY",
            environment.get("SSL_VERIFY").map_or("", String::as_str),
        )? {
            if value {
                Verification::Enabled
            } else {
                Verification::Disabled
            }
        } else {
            Verification::Enabled
        };
        let security_level = environment.get("SSL_SECURITY_LEVEL").cloned();
        if security_level.is_some() {
            return Err(TlsPolicyError::SecurityLevel);
        }
        Ok(Self {
            verification,
            certificate: environment.get("SSL_CERTIFICATE").cloned(),
            http2: betterlite_env::flag(
                "LITELLM_HTTP2",
                environment.get("LITELLM_HTTP2").map_or("", String::as_str),
            )?
            .unwrap_or(false),
            ecdh_curve: environment.get("SSL_ECDH_CURVE").cloned(),
            security_level,
            user_agent: environment
                .get("LITELLM_USER_AGENT")
                .cloned()
                .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION"))),
        })
    }
}

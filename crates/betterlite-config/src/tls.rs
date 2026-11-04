use std::collections::BTreeMap;

use crate::types::Config;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(untagged)]
pub enum VerifySetting {
    Bool(bool),
    Path(String),
}

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
        let verification = resolve_verification(config, environment, deployment_ssl_verify);
        let security_level = string_setting(
            environment,
            "SSL_SECURITY_LEVEL",
            config.litellm_settings.ssl_security_level.as_ref(),
        );
        if security_level.is_some() {
            return Err(TlsPolicyError::SecurityLevel);
        }
        let effective_verification =
            resolve_ca_file(verification, environment.get("SSL_CERT_FILE"));
        Ok(Self {
            ca_file: match &effective_verification {
                Verification::CaFile(path) => Some(path.clone()),
                Verification::Enabled | Verification::Disabled => None,
            },
            verification: effective_verification,
            certificate: string_setting(
                environment,
                "SSL_CERTIFICATE",
                config.litellm_settings.ssl_certificate.as_ref(),
            ),
            http2: config.litellm_settings.http2 == Some(true)
                || betterlite_env::flag(
                    "LITELLM_HTTP2",
                    environment.get("LITELLM_HTTP2").map_or("", String::as_str),
                )?
                .unwrap_or(false),
            ecdh_curve: string_setting(
                environment,
                "SSL_ECDH_CURVE",
                config.litellm_settings.ssl_ecdh_curve.as_ref(),
            ),
            user_agent: environment
                .get("LITELLM_USER_AGENT")
                .cloned()
                .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION"))),
        })
    }
}

fn string_setting(
    environment: &BTreeMap<String, String>,
    name: &str,
    fallback: Option<&String>,
) -> Option<String> {
    environment
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| fallback.cloned())
}

fn resolve_ca_file(verification: Verification, configured_path: Option<&String>) -> Verification {
    match configured_path.filter(|path_text| !path_text.is_empty()) {
        Some(path_text) if verification == Verification::Enabled => {
            Verification::CaFile(path_text.clone())
        }
        Some(_) | None => verification,
    }
}

fn resolve_verification(
    config: &Config,
    environment: &BTreeMap<String, String>,
    deployment_ssl_verify: Option<bool>,
) -> Verification {
    if let Some(value) = deployment_ssl_verify {
        return if value {
            Verification::Enabled
        } else {
            Verification::Disabled
        };
    }
    let value = environment
        .get("SSL_VERIFY")
        .filter(|value| !value.is_empty())
        .map(|value| VerifySetting::Path(value.clone()))
        .or_else(|| config.litellm_settings.ssl_verify.clone());
    match value {
        Some(VerifySetting::Bool(true)) | None => Verification::Enabled,
        Some(VerifySetting::Bool(false)) => Verification::Disabled,
        Some(VerifySetting::Path(path)) => match betterlite_env::flag("SSL_VERIFY", &path) {
            Ok(Some(true)) => Verification::Enabled,
            Ok(Some(false)) => Verification::Disabled,
            Ok(None) | Err(_) => Verification::CaFile(path),
        },
    }
}

mod tests;

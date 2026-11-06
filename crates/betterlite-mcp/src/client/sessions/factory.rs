use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::tls::{TlsPolicy, Verification};
use serde::{Deserialize, Serialize};
use url::Url;

pub const DEFAULT_CREDENTIAL_HEADER: &str = "Authorization";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsVerification {
    Enabled,
    Disabled,
    CaFile,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TlsSettings {
    pub verification: TlsVerification,
    pub ca_file: Option<String>,
}

impl TlsSettings {
    #[must_use]
    pub fn from_policy(policy: &TlsPolicy) -> Self {
        match &policy.verification {
            Verification::Enabled => Self {
                verification: TlsVerification::Enabled,
                ca_file: None,
            },
            Verification::Disabled => Self {
                verification: TlsVerification::Disabled,
                ca_file: None,
            },
            Verification::CaFile(path) => Self {
                verification: TlsVerification::CaFile,
                ca_file: Some(path.clone()),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    None,
    Explicit,
    Resolved,
    Aws,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RedirectGuard {
    pub configured_url: String,
    pub slot: String,
}

impl RedirectGuard {
    // HTTP clients strip Authorization across origins themselves, so only a credential in a configured slot needs a guard
    #[must_use]
    pub fn new(configured_url: &str, slot: Option<&str>) -> Option<Self> {
        let credential = slot?;
        if configured_url.is_empty() || credential.eq_ignore_ascii_case(DEFAULT_CREDENTIAL_HEADER) {
            return None;
        }
        Some(Self {
            configured_url: configured_url.to_owned(),
            slot: credential.to_owned(),
        })
    }

    // The credential leaves the configured origin with the redirect when the slot survives, so it is dropped first
    pub fn apply(&self, request_url: &str, headers: &mut BTreeMap<String, String>) {
        let crossing = match (Url::parse(&self.configured_url), Url::parse(request_url)) {
            (Ok(configured), Ok(target)) => crosses_origin(&configured, &target),
            _ => true,
        };
        if crossing {
            headers.retain(|name, _| !name.eq_ignore_ascii_case(&self.slot));
        }
    }
}

fn crosses_origin(configured: &Url, target: &Url) -> bool {
    let same_origin = configured.scheme() == target.scheme()
        && configured.host_str() == target.host_str()
        && configured.port_or_known_default() == target.port_or_known_default();
    if same_origin {
        return false;
    }
    let upgrade = configured.host_str() == target.host_str()
        && configured.scheme() == "http"
        && configured.port_or_known_default() == Some(80)
        && target.scheme() == "https"
        && target.port_or_known_default() == Some(443);
    !upgrade
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct HttpClientConfig {
    pub headers: BTreeMap<String, String>,
    pub timeout: Duration,
    pub tls: TlsSettings,
    pub auth: AuthSource,
    pub follow_redirects: bool,
    pub redirect_guard: Option<RedirectGuard>,
    pub capture_error_responses: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseRequest {
    pub url: String,
    pub timeout: Duration,
    pub headers: BTreeMap<String, String>,
    pub client: HttpClientConfig,
}

use thiserror::Error;
use tracing::warn;

use crate::trusted_network::cidr::{
    CidrRanges, ip_in_networks, normalize_cidr_ranges, parse_normalized,
};
use crate::trusted_network::request::NetworkRequest;
use crate::trusted_network::settings::{TrustedNetworkSettings, TrustedProxyConfig};

pub const TRUSTED_PROXY_RANGES_KEY: &str = "trusted_proxy_ranges";
pub const MCP_TRUSTED_PROXY_RANGES_KEY: &str = "mcp_trusted_proxy_ranges";

// user_api_key_auth._resolve_request_principal: X-Forwarded-For is trusted exactly when ranges are configured.
#[must_use]
pub fn proxy_config_from_ranges(configured: Option<&CidrRanges>) -> TrustedProxyConfig {
    let trusted_proxy_cidrs = normalize_cidr_ranges(configured);
    TrustedProxyConfig {
        use_forwarded_for: !trusted_proxy_cidrs.is_empty(),
        trusted_proxy_cidrs,
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TrustedProxyRequirementError {
    #[error(
        "{feature} requires general_settings.{setting} before trusting identity headers from an upstream proxy."
    )]
    NotConfigured { feature: String, setting: String },
    #[error(
        "{feature} only accepts identity headers from configured trusted proxy ranges. Direct client IP {direct} is not trusted."
    )]
    Untrusted { feature: String, direct: String },
}

// trusted_proxy_utils.require_trusted_proxy_request: the direct TCP peer ships the identity headers.
pub fn require_trusted_proxy_request(
    request: &NetworkRequest<'_>,
    settings: &TrustedNetworkSettings,
    feature_name: &str,
    setting_name: &str,
) -> Result<(), TrustedProxyRequirementError> {
    let configured = match setting_name {
        TRUSTED_PROXY_RANGES_KEY => settings.trusted_proxy_ranges.clone(),
        MCP_TRUSTED_PROXY_RANGES_KEY => settings
            .mcp_trusted_proxy_ranges
            .clone()
            .map(CidrRanges::List),
        _ => None,
    };
    let networks = parse_normalized(&normalize_cidr_ranges(configured.as_ref()), setting_name);
    if networks.is_empty() {
        return Err(TrustedProxyRequirementError::NotConfigured {
            feature: feature_name.to_owned(),
            setting: setting_name.to_owned(),
        });
    }
    if !ip_in_networks(request.peer, &networks) {
        warn!(
            feature = feature_name,
            direct = ?request.peer,
            "rejected identity headers from an untrusted direct client"
        );
        // the message embeds Python's repr of the peer: quoted for a host string, bare None when absent
        let direct = match request.peer {
            Some(host) => format!("'{host}'"),
            None => "None".to_owned(),
        };
        return Err(TrustedProxyRequirementError::Untrusted {
            feature: feature_name.to_owned(),
            direct,
        });
    }
    Ok(())
}

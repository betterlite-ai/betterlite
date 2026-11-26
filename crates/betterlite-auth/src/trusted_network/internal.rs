use std::net::IpAddr;

use ipnet::IpNet;
use tracing::warn;

use crate::trusted_network::cidr::network_from_text;

// IPAddressUtils._DEFAULT_INTERNAL_NETWORKS
pub const DEFAULT_INTERNAL_CIDRS: [&str; 6] = [
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "127.0.0.0/8",
    "::1/128",
    "fc00::/7",
];

#[must_use]
pub fn default_internal_networks() -> Vec<IpNet> {
    DEFAULT_INTERNAL_CIDRS
        .iter()
        .filter_map(|text| network_from_text(text))
        .collect()
}

// IPAddressUtils.parse_internal_networks: an empty or unusable configuration falls back to the defaults.
#[must_use]
pub fn parse_internal_networks(configured: Option<&[String]>) -> Vec<IpNet> {
    let Some(ranges) = configured else {
        return default_internal_networks();
    };
    let networks = parse_verbatim(ranges, "mcp_internal_ip_ranges");
    if networks.is_empty() {
        return default_internal_networks();
    }
    networks
}

// IPAddressUtils.parse_trusted_proxy_networks: unconfigured means X-Forwarded-For is never trusted.
#[must_use]
pub fn parse_trusted_proxy_networks(configured: Option<&[String]>) -> Vec<IpNet> {
    let Some(ranges) = configured else {
        return Vec::new();
    };
    parse_verbatim(ranges, "mcp_trusted_proxy_ranges")
}

// IPAddressUtils.is_trusted_proxy: the direct peer only, and no IPv4-mapped normalization here.
#[must_use]
pub fn is_trusted_proxy(proxy_ip: Option<&str>, trusted_networks: &[IpNet]) -> bool {
    let Some(proxy_text) = proxy_ip else {
        return false;
    };
    if proxy_text.is_empty() {
        return false;
    }
    if trusted_networks.is_empty() {
        return false;
    }
    let Ok(address) = proxy_text.trim().parse::<IpAddr>() else {
        return false;
    };
    trusted_networks
        .iter()
        .any(|network| network.contains(&address))
}

// IPAddressUtils.is_internal_ip: the leftmost entry of a comma chain, and the defaults when none parse.
#[must_use]
pub fn is_internal_ip(client_ip: Option<&str>, internal_networks: Option<&[IpNet]>) -> bool {
    let Some(client_text) = client_ip else {
        return false;
    };
    if client_text.is_empty() {
        return false;
    }
    let leftmost = match client_text.split_once(',') {
        Some((first, _)) => first.trim(),
        None => client_text,
    };
    let Ok(address) = leftmost.trim().parse::<IpAddr>() else {
        return false;
    };
    match internal_networks {
        Some(networks) if !networks.is_empty() => {
            networks.iter().any(|network| network.contains(&address))
        }
        Some(_) | None => default_internal_networks()
            .iter()
            .any(|network| network.contains(&address)),
    }
}

// The internal parsers pass each configured string through as written; only the XFF parser trims.
fn parse_verbatim(texts: &[String], setting_name: &str) -> Vec<IpNet> {
    let mut networks = Vec::with_capacity(texts.len());
    for text in texts {
        if let Some(network) = network_from_text(text) {
            networks.push(network);
        } else {
            warn!("Invalid CIDR in {setting_name}: {text}, skipping");
        }
    }
    networks
}

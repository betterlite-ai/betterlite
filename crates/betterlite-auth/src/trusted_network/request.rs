use std::net::IpAddr;

use tracing::{error, warn};

use crate::auth_identity::records::NetworkContext;
use crate::trusted_network::cidr::{ip_in_networks, parse_normalized};
use crate::trusted_network::internal::{is_trusted_proxy, parse_trusted_proxy_networks};
use crate::trusted_network::settings::{TrustedNetworkSettings, TrustedProxyConfig};

#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkRequest<'request> {
    pub peer: Option<&'request str>,
    pub x_forwarded_for: Option<&'request str>,
    pub x_forwarded_proto: Option<&'request str>,
    pub host: Option<&'request str>,
    pub scheme: &'request str,
}

// IPAddressUtils._HopCountSetting: absent keeps the legacy path, an unusable value fails closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedHopCount {
    Unset,
    Invalid,
    Count(i64),
}

// IPAddressUtils._resolve_num_trusted_hops: anything below one is unusable, never the legacy path.
#[must_use]
pub fn resolve_num_trusted_hops(configured: Option<i64>) -> TrustedHopCount {
    match configured {
        None => TrustedHopCount::Unset,
        Some(count) if count < 1 => {
            warn!(
                count,
                "mcp_xff_num_trusted_hops is below the minimum of 1; failing closed for MCP client IP resolution"
            );
            TrustedHopCount::Invalid
        }
        Some(count) => TrustedHopCount::Count(count),
    }
}

// IPAddressUtils.extract_client_ip_from_xff_hops: the Nth entry from the right, never the spoofable left end.
#[must_use]
pub fn client_ip_from_xff(xff_header: &str, num_trusted_hops: i64) -> Option<String> {
    if num_trusted_hops < 1 {
        return None;
    }
    let count = usize::try_from(num_trusted_hops).ok()?;
    let entries: Vec<&str> = xff_header
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.len() < count {
        return None;
    }
    let candidate = entries.get(entries.len().checked_sub(count)?)?;
    if candidate.parse::<IpAddr>().is_ok() {
        return Some((*candidate).to_owned());
    }
    None
}

// IPAddressUtils.is_request_from_trusted_proxy: XFF is trusted only from a configured proxy CIDR.
#[must_use]
pub fn is_request_from_trusted_proxy(
    request: &NetworkRequest<'_>,
    settings: &TrustedNetworkSettings,
) -> bool {
    if !settings.use_x_forwarded_for {
        return false;
    }
    match settings.mcp_trusted_proxy_ranges.as_deref() {
        Some(ranges) if !ranges.is_empty() => {
            is_trusted_proxy(request.peer, &parse_trusted_proxy_networks(Some(ranges)))
        }
        Some(_) | None => {
            warn!(
                "mcp_trusted_proxy_ranges is not configured, so X-Forwarded-* headers are not trusted"
            );
            false
        }
    }
}

// IPAddressUtils.is_request_https: PROXY_BASE_URL, then a trusted X-Forwarded-Proto, then the request scheme.
#[must_use]
pub fn is_request_https(request: &NetworkRequest<'_>, settings: &TrustedNetworkSettings) -> bool {
    let configured_base_url = settings
        .proxy_base_url
        .as_deref()
        .unwrap_or_default()
        .trim();
    if !configured_base_url.is_empty() {
        return url_scheme(configured_base_url).eq_ignore_ascii_case("https");
    }
    if is_request_from_trusted_proxy(request, settings)
        && let Some(forwarded_proto) = request.x_forwarded_proto
        && !forwarded_proto.is_empty()
    {
        let first = forwarded_proto.split(',').next().unwrap_or_default();
        return first.trim().eq_ignore_ascii_case("https");
    }
    request.scheme == "https"
}

// IPAddressUtils.get_mcp_client_ip: the MCP access-control client address, or "" when it must fail closed.
#[must_use]
pub fn get_mcp_client_ip(
    request: &NetworkRequest<'_>,
    settings: &TrustedNetworkSettings,
) -> Option<String> {
    let use_xff = settings.use_x_forwarded_for;
    if !use_xff && request.x_forwarded_for.is_some() {
        error!(
            "received an X-Forwarded-For header with use_x_forwarded_for disabled; the direct peer is used for MCP access control"
        );
    }
    if use_xff && let Some(forwarded_for) = request.x_forwarded_for {
        if !is_request_from_trusted_proxy(request, settings) {
            return untrusted_forwarded_client_ip(request, settings);
        }
        return Some(trusted_forwarded_client_ip(forwarded_for, settings));
    }
    Some(request_ip_address(request, use_xff))
}

// IPAddressUtils.get_mcp_client_ip's untrusted branch: the direct peer, or a missing peer fails closed.
fn untrusted_forwarded_client_ip(
    request: &NetworkRequest<'_>,
    settings: &TrustedNetworkSettings,
) -> Option<String> {
    let direct_ip = request.peer;
    if settings
        .mcp_trusted_proxy_ranges
        .as_deref()
        .is_some_and(|ranges| !ranges.is_empty())
    {
        warn!(
            direct = ?direct_ip,
            "X-Forwarded-For header from an untrusted IP, ignoring"
        );
        return direct_ip.map(str::to_owned);
    }
    // XFF enabled without trusted ranges: the peer is usually the proxy itself, so fail closed
    Some(String::new())
}

// the trusted branch: the Nth hop from the right, or the legacy whole header when the count is unset
fn trusted_forwarded_client_ip(forwarded_for: &str, settings: &TrustedNetworkSettings) -> String {
    match resolve_num_trusted_hops(settings.mcp_xff_num_trusted_hops) {
        TrustedHopCount::Invalid => String::new(),
        TrustedHopCount::Count(count) => {
            let Some(client_ip) = client_ip_from_xff(forwarded_for, count) else {
                warn!(
                    hops = count,
                    "X-Forwarded-For chain has fewer than mcp_xff_num_trusted_hops entries or an invalid address; failing closed"
                );
                return String::new();
            };
            client_ip
        }
        TrustedHopCount::Unset => forwarded_for.to_owned(),
    }
}

// auth_utils._get_request_ip_address: the whole header when XFF is on, else the direct peer, else "".
fn request_ip_address(request: &NetworkRequest<'_>, use_x_forwarded_for: bool) -> String {
    if use_x_forwarded_for && let Some(forwarded_for) = request.x_forwarded_for {
        return forwarded_for.to_owned();
    }
    request.peer.unwrap_or_default().to_owned()
}

// network.resolve_client_ip: walk the chain right-to-left and return the first hop that is not a proxy.
#[must_use]
pub fn resolve_client_ip(
    request: &NetworkRequest<'_>,
    config: &TrustedProxyConfig,
) -> (Option<String>, bool) {
    let peer = request.peer;
    let networks = parse_normalized(&config.trusted_proxy_cidrs, "trusted_proxy_cidrs");
    if !config.use_forwarded_for || !ip_in_networks(peer, &networks) {
        return (peer.map(str::to_owned), false);
    }
    let forwarded = request.x_forwarded_for.unwrap_or_default();
    let hops: Vec<&str> = forwarded.split(',').map(str::trim).collect();
    for hop in hops.iter().rev() {
        if hop.parse::<IpAddr>().is_ok() && !ip_in_networks(Some(*hop), &networks) {
            return (Some((*hop).to_owned()), true);
        }
    }
    (peer.map(str::to_owned), true)
}

// network.resolve_network_context: the resolved client, the Host header and whether XFF was trusted.
#[must_use]
pub fn resolve_network_context(
    request: &NetworkRequest<'_>,
    config: &TrustedProxyConfig,
) -> NetworkContext {
    let (client_ip, via_trusted_proxy) = resolve_client_ip(request, config);
    NetworkContext {
        client_ip,
        host: request.host.map(str::to_owned),
        via_trusted_proxy,
    }
}

// urllib.parse.urlparse(url).scheme: the text before ':' when urlparse reads a scheme there, as written.
pub(crate) fn url_scheme(url: &str) -> &str {
    let Some((scheme, _)) = url.split_once(':') else {
        return "";
    };
    let mut characters = scheme.chars();
    if !characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
    {
        return "";
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
    {
        return "";
    }
    scheme
}

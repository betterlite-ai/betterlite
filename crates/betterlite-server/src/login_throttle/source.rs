use std::net::IpAddr;
use std::slice::from_ref;

use betterlite_auth::trusted_network::cidr::parse_normalized;
use betterlite_config::sections::login_throttle::{
    OverrideLimit, ProxyTopology, SOURCE_LIMIT_OVERRIDES_KEY, Settings, SourceOverride,
    SourceOverrides, TRUSTED_PROXY_RANGES_KEY,
};
use ipnet::IpNet;
use tracing::warn;

pub const IPV6_SOURCE_PREFIX_LENGTH: u8 = 64;
pub const EXEMPT: i64 = 0;

// declared_proxy_ranges: only a topology whose every entry parses makes the source address trustworthy
#[must_use]
pub fn declared_proxy_ranges(topology: &ProxyTopology) -> Option<Vec<String>> {
    let ProxyTopology::Entries(entries) = topology else {
        return None;
    };
    for entry in entries {
        if parse_network(entry, TRUSTED_PROXY_RANGES_KEY).is_none() {
            return None;
        }
    }
    Some(entries.clone())
}

// source_group: IPv4 as is, IPv6 by its /64, an unparsable address by its literal text
#[must_use]
pub fn source_group(client_ip: &str) -> String {
    match parse_mapped(client_ip) {
        None => client_ip.to_owned(),
        Some(IpAddr::V4(version_four)) => version_four.to_string(),
        Some(IpAddr::V6(version_six)) => {
            IpNet::new(IpAddr::V6(version_six), IPV6_SOURCE_PREFIX_LENGTH).map_or_else(
                |_error| client_ip.to_owned(),
                |network| network.trunc().to_string(),
            )
        }
    }
}

// _source_limit: the most specific containing override, then an exemption, then the higher limit
#[must_use]
pub fn source_limit(settings: &Settings, client_ip: &str) -> i64 {
    let default = settings.source_limit;
    match &settings.source_limit_overrides {
        SourceOverrides::Absent => default,
        // the config section already warned about the shape; the throttle only uses the default
        SourceOverrides::Invalid => default,
        SourceOverrides::Mapping(entries) => source_override(entries, client_ip, default),
    }
}

// only a matching range evaluates its value, so an invalid one warns once that address reaches it
fn source_override(entries: &[SourceOverride], client_ip: &str, default: i64) -> i64 {
    let Some(address) = parse_mapped(client_ip) else {
        return default;
    };
    let mut best: Option<(u8, bool, i64)> = None;
    for entry in entries {
        let Some(network) = parse_network(&entry.range, SOURCE_LIMIT_OVERRIDES_KEY) else {
            continue;
        };
        if !network.contains(&address) {
            continue;
        }
        let limit = match &entry.limit {
            OverrideLimit::Exempt => EXEMPT,
            OverrideLimit::Limit(value) => *value,
            OverrideLimit::Invalid => {
                let range = &entry.range;
                warn!("Invalid {SOURCE_LIMIT_OVERRIDES_KEY} value for {range}; using {default}");
                default
            }
        };
        let precedence = (network.prefix_len(), limit == EXEMPT, limit);
        best = best.max(Some(precedence));
    }
    best.map_or(default, |(_prefix, _exempt, limit)| limit)
}

// _parse_network with B743's rules: strict=false truncation and either IPv4 mask form
fn parse_network(text: &str, setting_name: &str) -> Option<IpNet> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        // a blank entry is a typo too, and parse_normalized drops blanks before it could warn
        warn!("Invalid address or range \"{trimmed}\" in {setting_name}; skipping");
        return None;
    }
    let owned = trimmed.to_owned();
    let mut networks = parse_normalized(from_ref(&owned), setting_name);
    networks.pop()
}

// ipaddress.ip_address, including one non-empty IPv6 scope id and no slash anywhere
fn parse_address(text: &str) -> Option<IpAddr> {
    if text.contains('/') {
        return None;
    }
    match text.split_once('%') {
        None => text.parse().ok(),
        Some((address, scope))
            if address.contains(':') && !scope.is_empty() && !scope.contains('%') =>
        {
            address.parse().ok()
        }
        Some(_) => None,
    }
}

// _parse_address: an IPv4-mapped IPv6 address is the IPv4 address it embeds
fn parse_mapped(text: &str) -> Option<IpAddr> {
    match parse_address(text)? {
        IpAddr::V4(version_four) => Some(IpAddr::V4(version_four)),
        IpAddr::V6(version_six) => Some(
            version_six
                .to_ipv4_mapped()
                .map_or(IpAddr::V6(version_six), IpAddr::V4),
        ),
    }
}

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
        parse_network(entry, TRUSTED_PROXY_RANGES_KEY)?;
    }
    Some(entries.clone())
}

// source_group: IPv4 as is, IPv6 by its /64, an unparsable address by its literal text
#[must_use]
pub fn source_group(client_ip: &str) -> String {
    let Some((address, scope)) = parse_address(client_ip) else {
        return client_ip.to_owned();
    };
    match address.to_canonical() {
        IpAddr::V4(version_four) => version_four.to_string(),
        IpAddr::V6(version_six) => {
            let Ok(network) = IpNet::new(IpAddr::V6(version_six), IPV6_SOURCE_PREFIX_LENGTH) else {
                return client_ip.to_owned();
            };
            let network_address = network.network();
            // ipaddress keeps an IPv6 scope id on the network address when the /64 leaves every host bit zero
            let suffix = if network_address == IpAddr::V6(version_six) {
                scope.map_or_else(String::new, |id| format!("%{id}"))
            } else {
                String::new()
            };
            format!("{network_address}{suffix}/{IPV6_SOURCE_PREFIX_LENGTH}")
        }
    }
}

// _source_limit: the most specific containing override, then an exemption, then the higher limit
#[must_use]
pub fn source_limit(settings: &Settings, client_ip: &str) -> i64 {
    let default = settings.source_limit;
    match &settings.source_limit_overrides {
        // the config section already warned about the shape; the throttle only uses the default
        SourceOverrides::Absent | SourceOverrides::Invalid => default,
        SourceOverrides::Mapping(entries) => source_override(entries, client_ip, default),
    }
}

// only a matching range evaluates its value, so an invalid one warns once that address reaches it
fn source_override(entries: &[SourceOverride], client_ip: &str, default: i64) -> i64 {
    let Some((parsed, _scope)) = parse_address(client_ip) else {
        return default;
    };
    let address = parsed.to_canonical();
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

// ipaddress.ip_address, including one non-empty IPv6 scope id and no slash anywhere; the scope comes back because a /64 network address prints it
fn parse_address(text: &str) -> Option<(IpAddr, Option<String>)> {
    if text.contains('/') {
        return None;
    }
    match text.split_once('%') {
        None => text.parse().ok().map(|address| (address, None)),
        Some((address, scope))
            if address.contains(':') && !scope.is_empty() && !scope.contains('%') =>
        {
            address
                .parse()
                .ok()
                .map(|parsed| (parsed, Some(scope.to_owned())))
        }
        Some(_) => None,
    }
}

use std::net::IpAddr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use tracing::warn;

// LiteLLM accepts a comma-separated string at a direct call site; the config boundary only ever builds the list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CidrRanges {
    Text(String),
    List(Vec<String>),
}

// network.normalize_cidr_ranges: trim every entry, drop the empty ones, split a comma-separated string.
#[must_use]
pub fn normalize_cidr_ranges(configured: Option<&CidrRanges>) -> Vec<String> {
    let Some(configured) = configured else {
        return Vec::new();
    };
    match configured {
        CidrRanges::Text(text) => text
            .split(',')
            .map(str::trim)
            .filter(|range| !range.is_empty())
            .map(str::to_owned)
            .collect(),
        CidrRanges::List(items) => items
            .iter()
            .map(|range| range.trim())
            .filter(|range| !range.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

// network.parse_trusted_proxy_ranges: an entry that is not an address or CIDR range is skipped with a warning.
#[must_use]
pub fn parse_trusted_proxy_ranges(
    configured: Option<&CidrRanges>,
    setting_name: &str,
) -> Vec<IpNet> {
    parse_normalized(&normalize_cidr_ranges(configured), setting_name)
}

// network.parse_trusted_proxy_ranges applied to an already normalized list, as resolve_client_ip does.
pub(crate) fn parse_normalized(texts: &[String], setting_name: &str) -> Vec<IpNet> {
    let mut networks = Vec::with_capacity(texts.len());
    for text in texts {
        let cidr = text.trim();
        if cidr.is_empty() {
            continue;
        }
        match network_from_text(cidr) {
            Some(network) => networks.push(network),
            None => warn!("Invalid CIDR in {setting_name}: {cidr}, skipping"),
        }
    }
    networks
}

// network.ip_in_networks: both the address and its IPv4-mapped form are checked against every network.
#[must_use]
pub fn ip_in_networks(client_ip: Option<&str>, networks: &[IpNet]) -> bool {
    let Some(client_ip) = client_ip else {
        return false;
    };
    if client_ip.is_empty() {
        return false;
    }
    if networks.is_empty() {
        return false;
    }
    let Ok(address) = client_ip.trim().parse::<IpAddr>() else {
        return false;
    };
    let unmapped = unmapped(address);
    networks
        .iter()
        .any(|network| network.contains(&address) || network.contains(&unmapped))
}

// network._is_valid_ip
pub(crate) fn is_valid_ip(value: &str) -> bool {
    value.parse::<IpAddr>().is_ok()
}

// network._unmapped: an IPv6 address in ::ffff:0:0/96 becomes the IPv4 address it embeds.
fn unmapped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(_) => address,
        IpAddr::V6(ipv6) => match ipv6.to_ipv4_mapped() {
            Some(ipv4) => IpAddr::V4(ipv4),
            None => address,
        },
    }
}

// ipaddress.ip_network(text, strict=False): an optional prefix length, a netmask, or no slash at all.
pub(crate) fn network_from_text(text: &str) -> Option<IpNet> {
    let (address_text, prefix_text) = match text.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (text, None),
    };
    let address: IpAddr = address_text.parse().ok()?;
    let network = match prefix_text {
        None => IpNet::new(address, address_width(address)).ok()?,
        Some(prefix) if prefix.chars().all(|character| character.is_ascii_digit()) => {
            let length: u8 = prefix.parse().ok()?;
            IpNet::new(address, length).ok()?
        }
        Some(prefix) => {
            let mask: IpAddr = prefix.parse().ok()?;
            IpNet::with_netmask(address, mask).ok()?
        }
    };
    // strict=False: the host bits of the supplied address are masked away
    Some(network.trunc())
}

const fn address_width(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

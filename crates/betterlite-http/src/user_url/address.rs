use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::{Ipv4Net, Ipv6Net};

use crate::user_url::split::parse_ipv6;

// Python 3.13 ipaddress: _IPv4Constants
const V4_PRIVATE: [Ipv4Net; 14] = [
    Ipv4Net::new_assert(Ipv4Addr::UNSPECIFIED, 8),
    Ipv4Net::new_assert(Ipv4Addr::new(10, 0, 0, 0), 8),
    Ipv4Net::new_assert(Ipv4Addr::new(127, 0, 0, 0), 8),
    Ipv4Net::new_assert(Ipv4Addr::new(169, 254, 0, 0), 16),
    Ipv4Net::new_assert(Ipv4Addr::new(172, 16, 0, 0), 12),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 0), 24),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 170), 31),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 2, 0), 24),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 168, 0, 0), 16),
    Ipv4Net::new_assert(Ipv4Addr::new(198, 18, 0, 0), 15),
    Ipv4Net::new_assert(Ipv4Addr::new(198, 51, 100, 0), 24),
    Ipv4Net::new_assert(Ipv4Addr::new(203, 0, 113, 0), 24),
    Ipv4Net::new_assert(Ipv4Addr::new(240, 0, 0, 0), 4),
    Ipv4Net::new_assert(Ipv4Addr::BROADCAST, 32),
];
const V4_PRIVATE_EXCEPTIONS: [Ipv4Net; 2] = [
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 9), 32),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 10), 32),
];
const V4_SHARED: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(100, 64, 0, 0), 10);
const V4_MULTICAST: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(224, 0, 0, 0), 4);

// Python 3.13 ipaddress: _IPv6Constants
const V6_PRIVATE: [Ipv6Net; 11] = [
    Ipv6Net::new_assert(Ipv6Addr::LOCALHOST, 128),
    Ipv6Net::new_assert(Ipv6Addr::UNSPECIFIED, 128),
    Ipv6Net::new_assert(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0, 0), 96),
    Ipv6Net::new_assert(Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0), 48),
    Ipv6Net::new_assert(Ipv6Addr::new(0x100, 0, 0, 0, 0, 0, 0, 0), 64),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2002, 0, 0, 0, 0, 0, 0, 0), 16),
    Ipv6Net::new_assert(Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
    Ipv6Net::new_assert(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
    Ipv6Net::new_assert(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
];
const V6_PRIVATE_EXCEPTIONS: [Ipv6Net; 6] = [
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 1, 0, 0, 0, 0, 0, 1), 128),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 1, 0, 0, 0, 0, 0, 2), 128),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 3, 0, 0, 0, 0, 0, 0), 32),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 4, 0x112, 0, 0, 0, 0, 0), 48),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 0x20, 0, 0, 0, 0, 0, 0), 28),
    Ipv6Net::new_assert(Ipv6Addr::new(0x2001, 0x30, 0, 0, 0, 0, 0, 0), 28),
];
const V6_MULTICAST: Ipv6Net = Ipv6Net::new_assert(Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8);

// LiteLLM's _CLOUD_METADATA_EXCEPTIONS: Azure Wire Server and the whole of 192.0.0.0/24
const CLOUD_EXCEPTIONS: [Ipv4Net; 2] = [
    Ipv4Net::new_assert(Ipv4Addr::new(168, 63, 129, 16), 32),
    Ipv4Net::new_assert(Ipv4Addr::new(192, 0, 0, 0), 24),
];

fn v4_is_private(address: Ipv4Addr) -> bool {
    V4_PRIVATE.iter().any(|network| network.contains(&address))
        && !V4_PRIVATE_EXCEPTIONS
            .iter()
            .any(|network| network.contains(&address))
}

fn v6_is_private(address: Ipv6Addr) -> bool {
    V6_PRIVATE.iter().any(|network| network.contains(&address))
        && !V6_PRIVATE_EXCEPTIONS
            .iter()
            .any(|network| network.contains(&address))
}

fn v4_is_blocked(address: Ipv4Addr) -> bool {
    let global = !V4_SHARED.contains(&address) && !v4_is_private(address);
    !global
        || V4_MULTICAST.contains(&address)
        || CLOUD_EXCEPTIONS
            .iter()
            .any(|network| network.contains(&address))
}

// LiteLLM's _is_blocked_ip, after an IPv4-mapped IPv6 address is unwrapped
#[must_use]
pub fn is_blocked_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4_is_blocked(v4),
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or_else(
            || v6_is_private(v6) || V6_MULTICAST.contains(&v6),
            v4_is_blocked,
        ),
    }
}

// An address that Python's ip_address does not parse is blocked
#[must_use]
pub fn is_blocked_text(address: &str) -> bool {
    match address.parse::<IpAddr>() {
        Ok(parsed) => is_blocked_ip(parsed),
        Err(_error) => parse_ipv6(address).is_none_or(|scoped| is_blocked_ip(IpAddr::V6(scoped))),
    }
}

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{ParseError, Url};

use super::error::CompresrTransportError;

mod tests;

const BLOCKED_METADATA_HOSTS: [&str; 4] = [
    "metadata.google.internal",
    "metadata.goog",
    "metadata.azure.com",
    "metadata.azure.internal",
];

const BLOCKED_METADATA_IPS: [IpAddr; 4] = [
    IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254)),
    IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200)),
    IpAddr::V4(Ipv4Addr::new(168, 63, 129, 16)),
];

pub fn validate_api_base(url: &str) -> Result<String, CompresrTransportError> {
    let scheme = url.split_once(':').map(|(found, _)| found);
    if !matches!(scheme, Some(found) if found.eq_ignore_ascii_case("http") || found.eq_ignore_ascii_case("https"))
    {
        return Err(CompresrTransportError::Scheme);
    }
    let parsed = Url::parse(url).map_err(|error| match error {
        ParseError::EmptyHost => CompresrTransportError::NoHost,
        ParseError::RelativeUrlWithoutBase => CompresrTransportError::Scheme,
        ParseError::IdnaError
        | ParseError::InvalidPort
        | ParseError::InvalidIpv4Address
        | ParseError::InvalidIpv6Address
        | ParseError::InvalidDomainCharacter
        | ParseError::RelativeUrlWithCannotBeABaseBase
        | ParseError::SetHostOnCannotBeABaseUrl
        | ParseError::Overflow => CompresrTransportError::InvalidApiBase,
    })?;
    let Some(host) = parsed.host_str() else {
        return Err(CompresrTransportError::NoHost);
    };
    // host_str keeps the brackets of an IPv6 literal, and LiteLLM lowercases the hostname before matching
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let literal = parse_ip_literal(&host);
    if BLOCKED_METADATA_HOSTS.contains(&host.as_str())
        || literal.is_some_and(|address| BLOCKED_METADATA_IPS.contains(&address))
    {
        return Err(CompresrTransportError::BlockedHost { host });
    }
    Ok(url.to_owned())
}

#[must_use]
pub fn parse_ip_literal(host: &str) -> Option<IpAddr> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return Some(map_ipv4_mapped(address));
    }
    let magnitude = parse_integer_literal(host)?;
    Some(map_ipv4_mapped(integer_to_ip(magnitude)))
}

fn map_ipv4_mapped(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(six) => six.to_ipv4_mapped().map_or(address, IpAddr::V4),
        IpAddr::V4(four) => IpAddr::V4(four),
    }
}

fn integer_to_ip(magnitude: u128) -> IpAddr {
    match u32::try_from(magnitude) {
        Ok(four) => IpAddr::V4(Ipv4Addr::from(four)),
        Err(_) => IpAddr::V6(Ipv6Addr::from(magnitude)),
    }
}

// Python's int(text, 0): an optional sign, 0x/0o/0b prefixes, single underscores between digits, and no leading zero in decimal
fn parse_integer_literal(text: &str) -> Option<u128> {
    let trimmed = text.trim();
    let unsigned = trimmed.strip_prefix('+').unwrap_or(trimmed);
    // ipaddress.ip_address refuses a negative magnitude, so a signed negative never names an address
    if unsigned.starts_with('-') || unsigned.starts_with('+') {
        return None;
    }
    let (radix, digits) = match unsigned.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (16, unsigned.get(2..)?),
        Some("0o") => (8, unsigned.get(2..)?),
        Some("0b") => (2, unsigned.get(2..)?),
        _ => (10, unsigned),
    };
    let cleaned = digits_without_underscores(digits)?;
    if radix == 10 && cleaned.len() > 1 && cleaned.starts_with('0') {
        return None;
    }
    u128::from_str_radix(&cleaned, radix).ok()
}

fn digits_without_underscores(digits: &str) -> Option<String> {
    if digits.is_empty() {
        return None;
    }
    let mut cleaned = String::with_capacity(digits.len());
    let mut after_underscore = true;
    for character in digits.chars() {
        if character == '_' {
            if after_underscore {
                return None;
            }
            after_underscore = true;
        } else {
            cleaned.push(character);
            after_underscore = false;
        }
    }
    (!after_underscore).then_some(cleaned)
}

use std::net::Ipv6Addr;
use std::str::FromStr as _;

use super::UrlParseError;

// CPython's hostname property: lowercase everything before an IPv6 zone and keep the zone as written.
#[must_use]
pub fn hostname(netloc: &str) -> Option<String> {
    let (host, _port) = hostinfo(netloc);
    if host.is_empty() {
        return None;
    }
    Some(host.split_once('%').map_or_else(
        || host.to_lowercase(),
        |(address, zone)| format!("{}%{zone}", address.to_lowercase()),
    ))
}

// CPython's port property, except that a signed port text parses here where the port property rejects it.
pub fn port(netloc: &str) -> Result<Option<u16>, UrlParseError> {
    let Some(text) = hostinfo(netloc).1 else {
        return Ok(None);
    };
    let value = text
        .parse::<u16>()
        .map_err(|_error| UrlParseError::InvalidPort)?;
    Ok(Some(value))
}

// CPython's int() for a base-10 string: surrounding whitespace, one sign, digits, and single underscores between them.
#[must_use]
pub fn int_value(text: &str) -> Option<i128> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    let mut value: i128 = 0;
    let mut after_digit = false;
    for character in digits.chars() {
        if character == '_' && after_digit {
            after_digit = false;
            continue;
        }
        let digit = character.to_digit(10)?;
        value = value.checked_mul(10)?.checked_add(i128::from(digit))?;
        after_digit = true;
    }
    if !after_digit {
        return None;
    }
    Some(if negative {
        value.checked_neg()?
    } else {
        value
    })
}

fn hostinfo(netloc: &str) -> (&str, Option<&str>) {
    let after_userinfo = netloc.rsplit_once('@').map_or(netloc, |(_user, rest)| rest);
    let Some((_before, bracketed)) = after_userinfo.split_once('[') else {
        let (host, port) = after_userinfo
            .split_once(':')
            .unwrap_or((after_userinfo, ""));
        return (host, (!port.is_empty()).then_some(port));
    };
    let (host, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
    let port = after.split_once(':').map_or("", |(_colon, port)| port);
    (host, (!port.is_empty()).then_some(port))
}

pub(super) fn validate_netloc(netloc: &str) -> Result<(), UrlParseError> {
    let open = netloc.contains('[');
    if open != netloc.contains(']') {
        return Err(UrlParseError::InvalidIpv6);
    }
    if !open {
        return Ok(());
    }
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_user, rest)| rest);
    let Some((before, bracketed)) = hostinfo.split_once('[') else {
        return Err(UrlParseError::InvalidIpv6);
    };
    let Some((host, after)) = bracketed.split_once(']') else {
        return Err(UrlParseError::InvalidIpv6);
    };
    let address = host
        .split_once('%')
        .map_or(host, |(address, _zone)| address);
    let shaped = before.is_empty()
        && (after.is_empty() || after.starts_with(':'))
        && !address.is_empty()
        && Ipv6Addr::from_str(address).is_ok();
    shaped.then_some(()).ok_or(UrlParseError::InvalidIpv6)
}

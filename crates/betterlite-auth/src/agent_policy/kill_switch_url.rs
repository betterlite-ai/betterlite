mod tests;

use serde::{Deserialize, Serialize};
use std::net::Ipv6Addr;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization as _;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct KillSwitchUrl(pub(super) String);

#[derive(Debug, Error)]
#[error("kill_switch.url must be an absolute http(s) URL")]
pub struct KillSwitchUrlError;

impl TryFrom<String> for KillSwitchUrl {
    type Error = KillSwitchUrlError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let candidate = value
            .trim_start_matches(|character| character <= ' ')
            .replace(['\t', '\r', '\n'], "");
        let Some((scheme, remainder)) = candidate.split_once(':') else {
            return Err(KillSwitchUrlError);
        };
        let Some(authority) = remainder.strip_prefix("//") else {
            return Err(KillSwitchUrlError);
        };
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
            || !has_valid_authority(authority)
        {
            return Err(KillSwitchUrlError);
        }
        Ok(Self(value))
    }
}

fn has_valid_authority(value: &str) -> bool {
    let authority = value.split(['/', '?', '#']).next().unwrap_or_default();
    let unseparated = authority.replace(['@', ':', '#', '?'], "");
    if authority.is_empty()
        || unseparated
            .nfkc()
            .any(|character| "/?#@:".contains(character))
    {
        return false;
    }
    if !authority.contains(['[', ']']) {
        return true;
    }
    if !authority.contains('[') || !authority.contains(']') {
        return false;
    }
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some((literal, suffix)) = bracketed.split_once(']') else {
            return false;
        };
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return false;
        }
        literal
    } else if host_port.contains('[') {
        return false;
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    valid_ip_literal(host)
}

fn valid_ip_literal(host: &str) -> bool {
    if let Some(future) = host.strip_prefix(['v', 'V']) {
        return future.split_once('.').is_some_and(|(version, address)| {
            !version.is_empty()
                && version.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !address.is_empty()
        });
    }
    let address = if let Some((address, scope)) = host.split_once('%') {
        if scope.is_empty() || scope.contains('%') {
            return false;
        }
        address
    } else {
        host
    };
    address.parse::<Ipv6Addr>().is_ok()
}

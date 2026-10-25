use std::net::{Ipv4Addr, Ipv6Addr};

use unicode_normalization::UnicodeNormalization as _;

use crate::settings::{PYTHON_SPACE, SettingsError};

fn check_bracketed_host(host: &str) -> Result<(), SettingsError> {
    if let Some(future) = host.strip_prefix('v') {
        let valid = future.split_once('.').is_some_and(|(digits, rest)| {
            !digits.is_empty()
                && digits.chars().all(|digit| digit.is_ascii_hexdigit())
                && !rest.is_empty()
        });
        if !valid {
            return Err(SettingsError::IpvFuture);
        }
        return Ok(());
    }
    if host.parse::<Ipv4Addr>().is_ok() {
        return Err(SettingsError::BracketedIpv4);
    }
    let address = match host.split_once('%') {
        Some((unscoped, zone)) => (!zone.is_empty() && !zone.contains('%')).then_some(unscoped),
        None => Some(host),
    };
    if address.is_none_or(|candidate| candidate.parse::<Ipv6Addr>().is_err()) {
        return Err(SettingsError::BracketedHost {
            host: host.to_owned(),
        });
    }
    Ok(())
}

fn check_netloc(netloc: &str) -> Result<(), SettingsError> {
    if netloc.contains('[') != netloc.contains(']') {
        return Err(SettingsError::Ipv6Url);
    }
    if netloc.contains('[') {
        let host_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
        let host = match host_and_port.split_once('[') {
            Some((before, bracketed)) => {
                let (host, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                if !before.is_empty() || !(port.is_empty() || port.starts_with(':')) {
                    return Err(SettingsError::Ipv6Url);
                }
                host
            }
            None => host_and_port
                .split_once(':')
                .map_or(host_and_port, |(host, _)| host),
        };
        check_bracketed_host(host)?;
    }
    let stripped: String = netloc
        .chars()
        .filter(|character| !"@:#?".contains(*character))
        .collect();
    if !netloc.is_ascii() && stripped.nfkc().any(|character| "/?#@:".contains(character)) {
        return Err(SettingsError::NetlocNormalization {
            netloc: netloc.to_owned(),
        });
    }
    Ok(())
}

fn has_host_without_credentials(netloc: &str) -> bool {
    let (userinfo, host_and_port) = netloc.rsplit_once('@').unwrap_or(("", netloc));
    let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let host = match host_and_port.split_once('[') {
        Some((_, bracketed)) => bracketed
            .split_once(']')
            .map_or(bracketed, |(host, _)| host),
        None => host_and_port
            .split_once(':')
            .map_or(host_and_port, |(host, _)| host),
    };
    !host.is_empty() && username.is_empty() && password.is_empty()
}

pub fn normalize_github_api_url(value: &str) -> Result<String, SettingsError> {
    let normalized = value.trim_matches(PYTHON_SPACE).trim_end_matches('/');
    if normalized.is_empty() {
        return Err(SettingsError::MissingGitHubUrl);
    }
    let url: String = normalized
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, rest) = match url.split_once(':') {
        Some((scheme, rest))
            if scheme.starts_with(|character: char| character.is_ascii_alphabetic())
                && scheme.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "+-.".contains(character)
                }) =>
        {
            (scheme.to_ascii_lowercase(), rest)
        }
        _ => (String::new(), url.as_str()),
    };
    let after_slashes = rest.strip_prefix("//").unwrap_or_default();
    let (netloc, tail) = after_slashes.split_at(
        after_slashes
            .find(['/', '?', '#'])
            .unwrap_or(after_slashes.len()),
    );
    check_netloc(netloc)?;
    let (before_fragment, fragment) = tail.split_once('#').unwrap_or((tail, ""));
    let query = before_fragment
        .split_once('?')
        .map_or("", |(_, query)| query);
    if scheme != "https"
        || !has_host_without_credentials(netloc)
        || !query.is_empty()
        || !fragment.is_empty()
    {
        return Err(SettingsError::GitHubUrl);
    }
    Ok(normalized.to_owned())
}

mod tests;

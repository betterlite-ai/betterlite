use std::net::Ipv6Addr;

// urlsplit(url).path, or None where urlsplit raises ValueError for a malformed authority
#[must_use]
pub fn url_path(url: &str) -> Option<String> {
    let cleaned: String = url
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let after_scheme = without_scheme(&cleaned);
    let rest = match after_scheme.strip_prefix("//") {
        Some(authority) => {
            let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
            let (netloc, path) = authority.split_at_checked(end)?;
            check_netloc(netloc)?;
            path
        }
        None => after_scheme,
    };
    let before_fragment = rest.split_once('#').map_or(rest, |(kept, _)| kept);
    let path = before_fragment
        .split_once('?')
        .map_or(before_fragment, |(kept, _)| kept);
    Some(path.to_owned())
}

// A scheme is an ASCII letter followed by letters, digits, "+", "-" or "."
fn without_scheme(url: &str) -> &str {
    let Some((scheme, rest)) = url.split_once(':') else {
        return url;
    };
    let mut characters = scheme.chars();
    let leads = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic());
    let schemed = characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'));
    if leads && schemed { rest } else { url }
}

fn check_netloc(netloc: &str) -> Option<()> {
    match (netloc.contains('['), netloc.contains(']')) {
        (false, false) => Some(()),
        (true, true) => check_bracketed(netloc),
        (true, false) | (false, true) => None,
    }
}

// _check_bracketed_netloc: nothing before the bracket, only a port after it, and a valid host inside
fn check_bracketed(netloc: &str) -> Option<()> {
    let host_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let hostname = match host_and_port.split_once('[') {
        Some((before, bracketed)) => {
            let (inside, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !before.is_empty() || !(port.is_empty() || port.starts_with(':')) {
                return None;
            }
            inside
        }
        None => host_and_port
            .split_once(':')
            .map_or(host_and_port, |(name, _)| name),
    };
    bracketed_host_is_valid(hostname).then_some(())
}

// An IPvFuture literal, or what ipaddress.ip_address reads as IPv6, scope id included
fn bracketed_host_is_valid(hostname: &str) -> bool {
    if let Some(future) = hostname.strip_prefix(['v', 'V']) {
        return future.split_once('.').is_some_and(|(version, rest)| {
            !version.is_empty()
                && version.chars().all(|digit| digit.is_ascii_hexdigit())
                && !rest.is_empty()
        });
    }
    let (address, scope) = hostname
        .split_once('%')
        .map_or((hostname, None), |(ip, zone)| (ip, Some(zone)));
    let scoped = scope.is_none_or(|zone| !zone.is_empty() && !zone.contains('%'));
    scoped && address.parse::<Ipv6Addr>().is_ok()
}

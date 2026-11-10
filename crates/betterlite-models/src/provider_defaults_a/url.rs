mod tests;

use std::net::Ipv6Addr;

use unicode_normalization::UnicodeNormalization as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlParts {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub params: String,
    pub query: String,
    pub fragment: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("invalid IPv6 URL")]
    InvalidIpv6,
    #[error("netloc contains invalid characters under NFKC normalization")]
    InvalidNetloc,
}

const SCHEME_EXTRA: [char; 3] = ['+', '-', '.'];
const USES_PARAMS: [&str; 16] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];
const USES_NETLOC: [&str; 26] = [
    "", "ftp", "http", "gopher", "nntp", "telnet", "imap", "wais", "file", "mms", "https", "shttp",
    "snews", "prospero", "rtsp", "rtsps", "rtspu", "rsync", "svn", "svn+ssh", "sftp", "nfs", "git",
    "git+ssh", "ws", "wss",
];

// Ports urllib.parse.urlparse: leading spaces dropped, tabs and newlines dropped, then the components split.
pub fn parse(url: &str) -> Result<UrlParts, UrlError> {
    let stripped =
        url.trim_start_matches(|character: char| matches!(character, '\u{0}'..='\u{20}'));
    let cleaned: String = stripped
        .chars()
        .filter(|character| !matches!(character, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, after_scheme) = split_scheme(&cleaned);
    let (netloc, after_authority) = split_authority(after_scheme)?;
    let (before_fragment, fragment) = after_authority
        .split_once('#')
        .unwrap_or((after_authority, ""));
    let (before_query, query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));
    let (path, params) = if USES_PARAMS.contains(&scheme.as_str()) {
        split_params(before_query)
    } else {
        (before_query, "")
    };
    Ok(UrlParts {
        scheme,
        netloc,
        path: path.to_owned(),
        params: params.to_owned(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    })
}

fn split_scheme(url: &str) -> (String, &str) {
    match url.split_once(':') {
        Some((head, tail)) if is_scheme(head) => (head.to_ascii_lowercase(), tail),
        _ => (String::new(), url),
    }
}

fn split_authority(url: &str) -> Result<(String, &str), UrlError> {
    let Some(after) = url.strip_prefix("//") else {
        return Ok((String::new(), url));
    };
    let delimiter = after
        .char_indices()
        .find(|(_, character)| matches!(character, '/' | '?' | '#'))
        .map_or(after.len(), |(index, _)| index);
    let (authority, remainder) = after.split_at_checked(delimiter).unwrap_or((after, ""));
    check_brackets(authority)?;
    check_netloc(authority)?;
    Ok((authority.to_owned(), remainder))
}

// Ports urllib.parse.urlunparse: pieces are rejoined as they were read, so host case and escapes survive.
#[must_use]
pub fn unparse(parts: &UrlParts) -> String {
    let mut path = parts.path.clone();
    if !parts.params.is_empty() {
        path = format!("{path};{}", parts.params);
    }
    let mut assembled = if !parts.netloc.is_empty() {
        if !path.is_empty() && !path.starts_with('/') {
            path.insert(0, '/');
        }
        format!("//{}{path}", parts.netloc)
    } else if path.starts_with("//")
        || (!parts.scheme.is_empty()
            && USES_NETLOC.contains(&parts.scheme.as_str())
            && (path.is_empty() || path.starts_with('/')))
    {
        format!("//{path}")
    } else {
        path
    };
    if !parts.scheme.is_empty() {
        assembled = format!("{}:{assembled}", parts.scheme);
    }
    if !parts.query.is_empty() {
        assembled = format!("{assembled}?{}", parts.query);
    }
    if !parts.fragment.is_empty() {
        assembled = format!("{assembled}#{}", parts.fragment);
    }
    assembled
}

impl UrlParts {
    #[must_use]
    pub fn hostname(&self) -> Option<String> {
        let hostinfo = self
            .netloc
            .rsplit_once('@')
            .map_or(self.netloc.as_str(), |(_, after)| after);
        let hostname = match hostinfo.split_once('[') {
            Some((_, bracketed)) => bracketed
                .split_once(']')
                .map_or(bracketed, |(host, _)| host),
            None => hostinfo.split_once(':').map_or(hostinfo, |(host, _)| host),
        };
        if hostname.is_empty() {
            return None;
        }
        match hostname.split_once('%') {
            Some((host, zone)) => Some(format!("{}%{zone}", host.to_lowercase())),
            None => Some(hostname.to_lowercase()),
        }
    }
}

fn is_scheme(candidate: &str) -> bool {
    let mut characters = candidate.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && characters
            .all(|character| character.is_ascii_alphanumeric() || SCHEME_EXTRA.contains(&character))
}

fn check_brackets(netloc: &str) -> Result<(), UrlError> {
    let open = netloc.contains('[');
    let close = netloc.contains(']');
    if open != close {
        return Err(UrlError::InvalidIpv6);
    }
    if !open {
        return Ok(());
    }
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, after)| after);
    let Some((before, bracketed)) = hostinfo.split_once('[') else {
        return Ok(());
    };
    if !before.is_empty() {
        return Err(UrlError::InvalidIpv6);
    }
    let (host, rest) = bracketed.split_once(']').unwrap_or((bracketed, ""));
    if !rest.is_empty() && !rest.starts_with(':') {
        return Err(UrlError::InvalidIpv6);
    }
    if is_bracketed_host(host) {
        Ok(())
    } else {
        Err(UrlError::InvalidIpv6)
    }
}

// urllib.parse rejects a netloc whose NFKC normalization reintroduces a URL delimiter.
fn check_netloc(netloc: &str) -> Result<(), UrlError> {
    if netloc.is_ascii() {
        return Ok(());
    }
    let stripped: String = netloc
        .chars()
        .filter(|character| !matches!(character, '@' | ':' | '#' | '?'))
        .collect();
    let normalized: String = stripped.nfkc().collect();
    if stripped != normalized && normalized.contains(['/', '?', '#', '@', ':']) {
        return Err(UrlError::InvalidNetloc);
    }
    Ok(())
}

// urllib.parse's ipaddress check also accepts an IPvFuture literal and an IPv6 scope id.
fn is_bracketed_host(host: &str) -> bool {
    if let Some(future) = host.strip_prefix('v') {
        let Some((hex, suffix)) = future.split_once('.') else {
            return false;
        };
        return !hex.is_empty()
            && hex.chars().all(|character| character.is_ascii_hexdigit())
            && !suffix.is_empty();
    }
    match host.split_once('%') {
        Some((address, zone)) => {
            !zone.is_empty() && !zone.contains('%') && address.parse::<Ipv6Addr>().is_ok()
        }
        None => host.parse::<Ipv6Addr>().is_ok(),
    }
}

fn split_params(path: &str) -> (&str, &str) {
    let from = path.rfind('/').unwrap_or_default();
    let Some((position, _)) = path.match_indices(';').find(|(index, _)| *index >= from) else {
        return (path, "");
    };
    let (head, tail) = path.split_at_checked(position).unwrap_or((path, ""));
    (head, tail.strip_prefix(';').unwrap_or(tail))
}

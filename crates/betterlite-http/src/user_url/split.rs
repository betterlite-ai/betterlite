use std::net::{IpAddr, Ipv6Addr};

use unicode_normalization::UnicodeNormalization as _;

use crate::user_url::error::UrlParseError;

// Python 3.13 urllib.parse.uses_params and uses_netloc
const USES_PARAMS: [&str; 16] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtsps", "rtspu",
    "sip", "sips", "mms", "sftp", "tel",
];
pub(super) const USES_NETLOC: [&str; 27] = [
    "",
    "ftp",
    "http",
    "gopher",
    "nntp",
    "telnet",
    "imap",
    "wais",
    "file",
    "mms",
    "https",
    "shttp",
    "snews",
    "prospero",
    "rtsp",
    "rtsps",
    "rtspu",
    "rsync",
    "svn",
    "svn+ssh",
    "sftp",
    "nfs",
    "git",
    "git+ssh",
    "ws",
    "wss",
    // Apple's scheme, written in two parts so the spell checker does not read it as "items"
    concat!("it", "ms-services"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub scheme: String,
    pub netloc: String,
    pub path: String,
    pub params: String,
    pub query: String,
    pub fragment: String,
}

// Python's ipaddress accepts a scope id after an IPv6 address; the zone text is not part of the address
#[must_use]
pub fn parse_ipv6(text: &str) -> Option<Ipv6Addr> {
    let (address, zone) = text.split_once('%').unwrap_or((text, "x"));
    if zone.is_empty() || zone.contains('%') {
        return None;
    }
    address.parse().ok()
}

fn check_bracketed_host(hostname: &str) -> Result<(), UrlParseError> {
    if hostname.starts_with(['v', 'V']) {
        let digits = hostname.get(1..).unwrap_or_default();
        let hex_len = digits.chars().take_while(char::is_ascii_hexdigit).count();
        let after_hex = digits
            .get(hex_len..)
            .and_then(|tail| tail.strip_prefix('.'));
        return match after_hex {
            Some(label) if hex_len > 0 && !label.is_empty() => Ok(()),
            _ => Err(UrlParseError::InvalidIpvFuture),
        };
    }
    if hostname
        .parse::<IpAddr>()
        .is_ok_and(|ip| IpAddr::is_ipv4(&ip))
    {
        return Err(UrlParseError::Ipv4InBrackets);
    }
    match parse_ipv6(hostname) {
        Some(_) => Ok(()),
        None => Err(UrlParseError::NotAnIpAddress {
            host: hostname.to_owned(),
        }),
    }
}

fn check_after_bracket(bracketed: &str) -> Result<(), UrlParseError> {
    match bracketed.split_once(']') {
        Some((hostname, "")) => check_bracketed_host(hostname),
        Some((hostname, after)) if after.starts_with(':') => check_bracketed_host(hostname),
        None => check_bracketed_host(bracketed),
        Some(_) => Err(UrlParseError::InvalidIpv6Url),
    }
}

fn check_bracketed_netloc(netloc: &str) -> Result<(), UrlParseError> {
    let host_and_port = netloc.rsplit_once('@').map_or(netloc, |(_, tail)| tail);
    match host_and_port.split_once('[') {
        None => check_bracketed_host(host_and_port.split(':').next().unwrap_or_default()),
        Some(("", bracketed)) => check_after_bracket(bracketed),
        Some(_) => Err(UrlParseError::InvalidIpv6Url),
    }
}

// Characters like U+2100 expand to 'a/c' under the NFKC step IDNA applies to a host
fn check_netloc(netloc: &str) -> Result<(), UrlParseError> {
    let stripped = netloc.replace(['@', ':', '#', '?'], "");
    let normalized: String = stripped.nfkc().collect();
    if stripped != normalized && normalized.contains(['/', '?', '#', '@', ':']) {
        return Err(UrlParseError::NetlocNfkc {
            netloc: netloc.to_owned(),
        });
    }
    Ok(())
}

// The scheme ahead of the first colon, when it has the shape of one
fn split_scheme(url: &str) -> Option<(String, &str)> {
    let (head, tail) = url.split_once(':')?;
    let starts_alpha = head
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic());
    let in_scheme =
        |letter: char| letter.is_ascii_alphanumeric() || matches!(letter, '+' | '-' | '.');
    (starts_alpha && head.chars().all(in_scheme)).then(|| (head.to_lowercase(), tail))
}

// The "//netloc" at the start of a reference, with its brackets checked
fn split_netloc(rest: &str) -> Result<(&str, &str), UrlParseError> {
    let Some(after_slashes) = rest.strip_prefix("//") else {
        return Ok(("", rest));
    };
    let end = after_slashes
        .find(['/', '?', '#'])
        .unwrap_or(after_slashes.len());
    let (netloc, tail) = after_slashes
        .split_at_checked(end)
        .unwrap_or((after_slashes, ""));
    if netloc.contains('[') != netloc.contains(']') {
        return Err(UrlParseError::InvalidIpv6Url);
    }
    if netloc.contains('[') {
        check_bracketed_netloc(netloc)?;
    }
    Ok((netloc, tail))
}

// Python 3.13 urlsplit, with the scheme argument fixed to the default for a URL without one
fn urlsplit(url: &str, default_scheme: &str) -> Result<Parsed, UrlParseError> {
    let cleaned: String = url
        .trim_start_matches(|letter: char| letter <= ' ')
        .chars()
        .filter(|letter| !matches!(letter, '\t' | '\r' | '\n'))
        .collect();
    let (scheme, after_scheme) =
        split_scheme(&cleaned).unwrap_or_else(|| (default_scheme.to_owned(), &cleaned));
    let (netloc, rest) = split_netloc(after_scheme)?;
    let (before_fragment, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (path, query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));
    check_netloc(netloc)?;
    Ok(Parsed {
        scheme,
        netloc: netloc.to_owned(),
        path: path.to_owned(),
        params: String::new(),
        query: query.to_owned(),
        fragment: fragment.to_owned(),
    })
}

// Python 3.13 urlparse: the ";params" of the last path segment split off for the schemes that use them
pub fn urlparse(url: &str, default_scheme: &str) -> Result<Parsed, UrlParseError> {
    let mut parsed = urlsplit(url, default_scheme)?;
    if USES_PARAMS.contains(&parsed.scheme.as_str()) && parsed.path.contains(';') {
        let last_segment = parsed.path.rfind('/').unwrap_or(0);
        let (head, tail) = parsed
            .path
            .split_at_checked(last_segment)
            .unwrap_or_default();
        if let Some((segment, params)) = tail.split_once(';') {
            parsed.params = params.to_owned();
            parsed.path = format!("{head}{segment}");
        }
    }
    Ok(parsed)
}

impl Parsed {
    // (hostname, port) as Python's _hostinfo splits them
    fn hostinfo(&self) -> (&str, &str) {
        let hostinfo = self
            .netloc
            .rsplit_once('@')
            .map_or(self.netloc.as_str(), |(_, tail)| tail);
        match hostinfo.split_once('[') {
            Some((_, bracketed)) => {
                let (hostname, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
                (hostname, after.split_once(':').map_or("", |(_, port)| port))
            }
            None => hostinfo.split_once(':').unwrap_or((hostinfo, "")),
        }
    }

    #[must_use]
    pub fn hostname(&self) -> Option<String> {
        let (hostname, _) = self.hostinfo();
        if hostname.is_empty() {
            return None;
        }
        Some(hostname.split_once('%').map_or_else(
            || hostname.to_lowercase(),
            |(name, scope)| format!("{}%{scope}", name.to_lowercase()),
        ))
    }

    pub fn port(&self) -> Result<Option<u16>, UrlParseError> {
        let (_, port) = self.hostinfo();
        if port.is_empty() {
            return Ok(None);
        }
        if !port.chars().all(|digit| digit.is_ascii_digit()) {
            return Err(UrlParseError::PortNotInteger {
                port: port.to_owned(),
            });
        }
        port.parse()
            .map(Some)
            .map_err(|_source| UrlParseError::PortOutOfRange)
    }

    // Python's urlunparse
    #[must_use]
    pub fn unparse(&self) -> String {
        let mut url = if self.params.is_empty() {
            self.path.clone()
        } else {
            format!("{};{}", self.path, self.params)
        };
        if self.netloc.is_empty() {
            let keeps_slashes = USES_NETLOC.contains(&self.scheme.as_str())
                && !self.scheme.is_empty()
                && (url.is_empty() || url.starts_with('/'));
            if url.starts_with("//") || keeps_slashes {
                url = format!("//{url}");
            }
        } else {
            if !url.is_empty() && !url.starts_with('/') {
                url.insert(0, '/');
            }
            url = format!("//{}{url}", self.netloc);
        }
        if !self.scheme.is_empty() {
            url = format!("{}:{url}", self.scheme);
        }
        if !self.query.is_empty() {
            url = format!("{url}?{}", self.query);
        }
        if !self.fragment.is_empty() {
            url = format!("{url}#{}", self.fragment);
        }
        url
    }
}

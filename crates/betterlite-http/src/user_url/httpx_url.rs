use std::borrow::Cow;
use std::net::Ipv4Addr;

use idna::AsciiDenyList;

use crate::user_url::error::InvalidUrl;
use crate::user_url::split::parse_ipv6;

const MAX_URL_LENGTH: usize = 0x0001_0000;
const HOST_ESCAPED: &str = " #/:<>?@[]^";

// The characters httpx leaves unescaped are the printable ASCII ones outside the WHATWG percent-encode set of the component
const QUERY_ESCAPED: &str = " \"#<>";
const FRAGMENT_ESCAPED: &str = " \"<>`";
const PATH_ESCAPED: &str = " \"#<>?`{}";
const USERINFO_ESCAPED: &str = " \"#<>?`{}/;=@[\\]^|";

// httpx 0.28.1 URL: the normalized form that urllib.parse.urljoin works on
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpxUrl {
    pub scheme: String,
    pub userinfo: String,
    pub host: String,
    pub port: Option<u64>,
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

// httpx quote(): a character outside printable ASCII or in `escaped` is percent-encoded as UTF-8, an existing %XX is kept
fn quote(text: &str, escaped: &str) -> String {
    let mut quoted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(letter) = rest.chars().next() {
        let escape = rest.get(..3).filter(|piece| {
            let mut chars = piece.chars();
            chars.next() == Some('%') && chars.all(|digit| digit.is_ascii_hexdigit())
        });
        let width = if escape.is_some() {
            3
        } else {
            letter.len_utf8()
        };
        let (taken, tail) = rest.split_at_checked(width).unwrap_or((rest, ""));
        if escape.is_some() || ((' '..='~').contains(&letter) && !escaped.contains(letter)) {
            quoted.push_str(taken);
        } else {
            quoted.extend(taken.bytes().map(|byte| format!("%{byte:02X}")));
        }
        rest = tail;
    }
    quoted
}

fn reject_control_characters(text: &str) -> Result<(), InvalidUrl> {
    let found = text
        .chars()
        .enumerate()
        .find(|(_, letter)| letter.is_ascii() && !(' '..='~').contains(letter));
    match found {
        None => Ok(()),
        Some((position, letter)) => Err(InvalidUrl::NonPrintable {
            character: match letter {
                '\t' => "'\\t'".to_owned(),
                '\r' => "'\\r'".to_owned(),
                '\n' => "'\\n'".to_owned(),
                other => format!("'\\x{:02x}'", u32::from(other)),
            },
            position,
        }),
    }
}

fn encode_host(host: &str) -> Result<String, InvalidUrl> {
    let dotted_digits = host.split('.').count() == 4
        && host
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|digit| digit.is_ascii_digit()));
    if host.is_empty() {
        Ok(String::new())
    } else if dotted_digits {
        host.parse::<Ipv4Addr>()
            .map(|_| host.to_owned())
            .map_err(|_source| InvalidUrl::Ipv4 {
                host: host.to_owned(),
            })
    } else if let Some(inner) = host
        .strip_prefix('[')
        .and_then(|tail| tail.strip_suffix(']'))
    {
        parse_ipv6(inner)
            .map(|_| inner.to_owned())
            .ok_or_else(|| InvalidUrl::Ipv6 {
                host: inner.to_owned(),
            })
    } else if host.is_ascii() {
        Ok(quote(&host.to_lowercase(), HOST_ESCAPED))
    } else {
        // IDNA2008 as the idna package does it; the idna crate maps by UTS 46, which agrees on lowercase input
        idna::domain_to_ascii_cow(host.to_lowercase().as_bytes(), AsciiDenyList::EMPTY)
            .map(Cow::into_owned)
            .map_err(|_source| InvalidUrl::Idna {
                host: host.to_owned(),
            })
    }
}

fn normalize_port(port: &str, scheme: &str) -> Result<Option<u64>, InvalidUrl> {
    if port.is_empty() {
        return Ok(None);
    }
    let number: u64 = port.parse().map_err(|_source| InvalidUrl::Port {
        port: port.to_owned(),
    })?;
    let default = match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    };
    Ok((Some(number) != default).then_some(number))
}

// RFC 3986 5.2.4 without the "starts with a slash" bookkeeping: httpx drops "." and ".." segments
fn normalize_path(path: &str) -> String {
    let mut output: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." => {}
            ".." => {
                if !output.is_empty() && output != [""] {
                    output.pop();
                }
            }
            other => output.push(other),
        }
    }
    output.join("/")
}

pub(super) fn validate_path(
    path: &str,
    scheme: &str,
    has_authority: bool,
) -> Result<(), InvalidUrl> {
    if has_authority && !path.is_empty() && !path.starts_with('/') {
        return Err(InvalidUrl::PathNotAbsolute);
    }
    if scheme.is_empty() && !has_authority {
        if path.starts_with("//") {
            return Err(InvalidUrl::RelativeDoubleSlash);
        }
        if path.starts_with(':') {
            return Err(InvalidUrl::RelativeColon);
        }
    }
    Ok(())
}

// httpx's URL_REGEX: [scheme ":"] ["//" authority] path ["?" query] ["#" fragment]
fn split_reference(url: &str) -> (&str, &str, &str, Option<&str>, Option<&str>) {
    let scheme_length =
        url.find(|letter: char| !(letter.is_ascii_alphanumeric() || "+.-".contains(letter)));
    let (scheme, rest) = match scheme_length.and_then(|end| url.split_at_checked(end)) {
        Some((name, tail))
            if tail.starts_with(':')
                && (name.is_empty()
                    || name.starts_with(|first: char| first.is_ascii_alphabetic())) =>
        {
            (name, tail.strip_prefix(':').unwrap_or(tail))
        }
        _ => ("", url),
    };
    let (authority, after_authority) = rest.strip_prefix("//").map_or(("", rest), |after| {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        after.split_at_checked(end).unwrap_or((after, ""))
    });
    let (before_fragment, fragment) = match after_authority.split_once('#') {
        Some((head, tail)) => (head, Some(tail)),
        None => (after_authority, None),
    };
    let (path, query) = match before_fragment.split_once('?') {
        Some((head, tail)) => (head, Some(tail)),
        None => (before_fragment, None),
    };
    (scheme, authority, path, query, fragment)
}

// httpx's AUTHORITY_REGEX: [userinfo "@"] host [":" port], a host in brackets running to the last "]"
fn split_authority(authority: &str) -> (&str, &str, &str) {
    let (userinfo, hostport) = authority.rsplit_once('@').unwrap_or(("", authority));
    let bracket_end = hostport.rfind(']').filter(|_end| hostport.starts_with('['));
    if let Some((host, after)) =
        bracket_end.and_then(|end| hostport.split_at_checked(end.saturating_add(1)))
    {
        return (userinfo, host, after.strip_prefix(':').unwrap_or(after));
    }
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, ""));
    (userinfo, host, port)
}

impl HttpxUrl {
    pub fn parse(url: &str) -> Result<Self, InvalidUrl> {
        if url.chars().count() > MAX_URL_LENGTH {
            return Err(InvalidUrl::TooLong);
        }
        reject_control_characters(url)?;
        let (raw_scheme, authority, raw_path, query, fragment) = split_reference(url);
        let (raw_userinfo, raw_host, raw_port) = split_authority(authority);
        let scheme = raw_scheme.to_lowercase();
        let userinfo = quote(raw_userinfo, USERINFO_ESCAPED);
        let host = encode_host(raw_host)?;
        let port = normalize_port(raw_port, raw_scheme)?;
        let has_authority = !userinfo.is_empty() || !host.is_empty() || port.is_some();
        validate_path(raw_path, &scheme, has_authority)?;
        let path = if scheme.is_empty() && !has_authority {
            raw_path.to_owned()
        } else {
            normalize_path(raw_path)
        };
        Ok(Self {
            scheme,
            userinfo,
            host,
            port,
            path: quote(&path, PATH_ESCAPED),
            query: query.map(|text| quote(text, QUERY_ESCAPED)),
            fragment: fragment.map(|text| quote(text, FRAGMENT_ESCAPED)),
        })
    }

    #[must_use]
    pub fn port_or_default(&self) -> Option<u64> {
        self.port.or(match self.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        })
    }

    // httpx's str(URL): scheme, "//" and the authority when there is one, then path, query and fragment
    #[must_use]
    pub fn href(&self) -> String {
        let (open, close) = if self.host.contains(':') {
            ("[", "]")
        } else {
            ("", "")
        };
        let at = if self.userinfo.is_empty() { "" } else { "@" };
        let port = self
            .port
            .map_or_else(String::new, |number| format!(":{number}"));
        let authority = format!("{}{at}{open}{}{close}{port}", self.userinfo, self.host);
        let scheme = if self.scheme.is_empty() {
            String::new()
        } else {
            format!("{}:", self.scheme)
        };
        let slashes = if authority.is_empty() { "" } else { "//" };
        let query = self
            .query
            .as_ref()
            .map_or_else(String::new, |text| format!("?{text}"));
        let fragment = self
            .fragment
            .as_ref()
            .map_or_else(String::new, |text| format!("#{text}"));
        format!("{scheme}{slashes}{authority}{}{query}{fragment}", self.path)
    }
}

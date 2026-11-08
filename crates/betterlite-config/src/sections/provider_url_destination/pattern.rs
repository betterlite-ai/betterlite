const MAX_ENTRY_CHARS: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedDestination {
    pub host: String,
    pub scheme: Option<Scheme>,
    pub port: Option<u16>,
}

fn split_scheme(entry: &str) -> Option<(Option<Scheme>, &str)> {
    let Some((name, rest)) = entry.split_once("://") else {
        return Some((None, entry));
    };
    let scheme = match name.to_ascii_lowercase().as_str() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        _ => return None,
    };
    Some((Some(scheme), rest))
}

fn split_authority(authority: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after {
            "" => None,
            tail => Some(tail.strip_prefix(':')?),
        };
        return Some((host, port));
    }
    match authority.split_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

fn parse_port(text: &str) -> Option<u16> {
    if !text.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl AllowedDestination {
    // LiteLLM's _parse_url_destination_allowlist_entry, with a malformed entry skipped instead of raising
    #[must_use]
    pub fn parse(entry: &str) -> Option<Self> {
        let text = entry.trim();
        if text.is_empty() || text.chars().count() > MAX_ENTRY_CHARS {
            return None;
        }
        let (scheme, rest) = split_scheme(text)?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = rest.get(..end)?;
        if authority.contains('@') {
            return None;
        }
        let (host, port_text) = split_authority(authority)?;
        if host.is_empty() {
            return None;
        }
        let port_number = match port_text {
            None | Some("") => None,
            Some(value) => Some(parse_port(value)?),
        };
        let port = match (scheme, port_number) {
            (Some(Scheme::Https), None) => Some(443),
            (Some(Scheme::Http), None) => Some(80),
            (_, explicit) => explicit,
        };
        Some(Self {
            host: host.to_lowercase().trim_end_matches('.').to_owned(),
            scheme,
            port,
        })
    }
}

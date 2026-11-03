#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerInfo {
    pub address: Option<String>,
    pub port: Option<u16>,
}

impl ServerInfo {
    pub fn from_api_base(value: Option<&str>) -> Option<Self> {
        let value = value?.trim();
        if value.is_empty() {
            return None;
        }
        let authority = value
            .split_once("://")
            .map(|(_, tail)| tail)
            .unwrap_or(value)
            .split('/')
            .next()?;
        let authority = authority.rsplit('@').next()?;
        let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
            let (host, suffix) = rest.split_once(']')?;
            (
                host,
                suffix
                    .strip_prefix(':')
                    .and_then(|port| port.parse::<u16>().ok()),
            )
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            (host, port.parse::<u16>().ok())
        } else {
            (authority, None)
        };
        if host.is_empty() {
            return None;
        }
        let scheme = value.split_once("://").map(|(scheme, _)| scheme);
        Some(Self {
            address: Some(host.to_owned()),
            port: port.or_else(|| match scheme {
                Some("https") => Some(443),
                Some("http") => Some(80),
                _ => None,
            }),
        })
    }
}

#[path = "server/tests.rs"]
mod tests;

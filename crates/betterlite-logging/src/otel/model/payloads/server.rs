#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerInfo {
    pub address: Option<String>,
    pub port: Option<u16>,
}

impl ServerInfo {
    #[must_use]
    pub fn from_api_base(api_base: Option<&str>) -> Option<Self> {
        let trimmed = api_base?.trim();
        if trimmed.is_empty() {
            return None;
        }
        let explicit_scheme = trimmed.split_once("://").map(|(scheme, _)| scheme);
        let parseable = if explicit_scheme.is_some() {
            trimmed.to_owned()
        } else {
            format!("http://{trimmed}")
        };
        let parsed = url::Url::parse(&parseable).ok()?;
        let host = parsed.host_str()?;
        Some(Self {
            address: Some(host.to_owned()),
            port: parsed.port().or(match explicit_scheme {
                Some("https") => Some(443),
                Some("http") => Some(80),
                _ => None,
            }),
        })
    }
}

#[path = "server/tests.rs"]
mod tests;

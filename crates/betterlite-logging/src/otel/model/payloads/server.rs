#[derive(serde::Serialize, Clone, Debug, Default, PartialEq, Eq)]
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
        let parseable = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("http://{trimmed}")
        };
        let raw_authority = parseable
            .split_once("://")
            .map_or(parseable.as_str(), |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()?;
        if raw_authority.is_empty() {
            return None;
        }
        url::Url::parse(&parseable).ok()?.host()?;
        let host_authority = raw_authority.rsplit('@').next()?;
        let (host, port) = if let Some(bracketed) = host_authority.strip_prefix('[') {
            let (address, suffix) = bracketed.split_once(']')?;
            (address, suffix.strip_prefix(':'))
        } else {
            host_authority
                .rsplit_once(':')
                .map_or((host_authority, None), |(address, port)| {
                    (address, Some(port))
                })
        };
        Some(Self {
            address: Some(host.to_lowercase()),
            port: port.and_then(|value| value.parse::<u16>().ok()),
        })
    }
}

#[path = "server/tests.rs"]
mod tests;

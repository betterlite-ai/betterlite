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
        let parseable = if trimmed.contains("://") {
            trimmed.to_owned()
        } else {
            format!("http://{trimmed}")
        };
        let parsed = url::Url::parse(&parseable).ok()?;
        let host = match parsed.host()? {
            url::Host::Ipv6(address) => address.to_string(),
            url::Host::Domain(domain) => domain.to_owned(),
            url::Host::Ipv4(address) => address.to_string(),
        };
        let raw_authority = parseable
            .split_once("://")
            .map_or(parseable.as_str(), |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()?;
        let host_authority = raw_authority.rsplit('@').next()?;
        let explicit_port = if host_authority.starts_with('[') {
            host_authority
                .find(']')
                .and_then(|end| end.checked_add(1))
                .and_then(|start| host_authority.get(start..))
                .and_then(|suffix| suffix.strip_prefix(':'))
        } else {
            host_authority.rsplit_once(':').map(|(_, port)| port)
        }
        .and_then(|port| port.parse::<u16>().ok());
        Some(Self {
            address: Some(host),
            port: explicit_port,
        })
    }
}

#[path = "server/tests.rs"]
mod tests;

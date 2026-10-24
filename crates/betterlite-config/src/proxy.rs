use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use netrc_rs::Netrc;
use percent_encoding::percent_decode_str;
use secrecy::SecretString;
use url::Url;

use crate::types::LiteLlmSettings;

#[derive(Debug, Clone)]
pub struct ProxyPolicy {
    pub no_proxy: Option<String>,
    pub primary: BTreeMap<String, ProxyRoute>,
    pub fallback: BTreeMap<String, ProxyRoute>,
    pub warnings: Vec<ProxyWarning>,
}

#[derive(Debug, Clone)]
pub enum ProxyRoute {
    Forward(Arc<ProxyServer>),
    Unsupported { scheme: String },
    Invalid { name: String },
}

#[derive(Debug)]
pub struct ProxyServer {
    pub host: String,
    pub port: u16,
    pub credentials: Option<ProxyCredentials>,
}

#[derive(Debug)]
pub struct ProxyCredentials {
    pub username: String,
    pub password: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyWarning {
    SecureProxyIgnored { name: String, scheme: String },
    NetrcUnusable { path: PathBuf },
}

impl ProxyPolicy {
    #[must_use]
    pub fn resolve(settings: &LiteLlmSettings, environment: &BTreeMap<String, String>) -> Self {
        let is_true = |name: &str| {
            environment
                .get(name)
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
        };
        let mut proxies = environment_proxies(environment);
        let no_proxy = proxies.remove("no");
        let mut primary = BTreeMap::new();
        for (name, value) in &proxies {
            let prefix = if value.contains("://") { "" } else { "http://" };
            let route = parse_route(name, &format!("{prefix}{value}"), None);
            primary.insert(name.clone(), route);
        }
        if settings.disable_aiohttp_trust_env || is_true("DISABLE_AIOHTTP_TRUST_ENV") {
            primary.clear();
        }
        let mut warnings = Vec::new();
        let fallback = if settings.aiohttp_trust_env || is_true("AIOHTTP_TRUST_ENV") {
            fallback_routes(&proxies, environment, &mut warnings)
        } else {
            BTreeMap::new()
        };
        Self {
            no_proxy,
            primary,
            fallback,
            warnings,
        }
    }

    #[must_use]
    pub fn route(&self, url: &Url) -> Option<&ProxyRoute> {
        // httpx hands LiteLLM an IPv6 host without its brackets
        let host = url.host_str().unwrap_or_default();
        if let Some(no_proxy) = &self.no_proxy
            && bypasses(no_proxy, host.trim_matches(['[', ']']))
        {
            return None;
        }
        self.primary
            .get(url.scheme())
            .or_else(|| self.primary.get("all"))
            .or_else(|| self.fallback.get(url.scheme()))
    }
}

fn environment_proxies(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let proxy_name = |name: &str| {
        let split = name.len().checked_sub(6)?;
        let suffix = name.get(split..)?;
        let prefix = name.get(..split)?;
        suffix
            .eq_ignore_ascii_case("_proxy")
            .then(|| (prefix.to_lowercase(), suffix == "_proxy"))
    };
    let mut proxies = BTreeMap::new();
    let mut lowercase = Vec::new();
    for (name, value) in environment {
        let Some((key, is_lowercase)) = proxy_name(name) else {
            continue;
        };
        if !value.is_empty() {
            proxies.insert(key.clone(), value.clone());
        }
        if is_lowercase {
            lowercase.push((key, value));
        }
    }
    // CPython drops HTTP_PROXY under CGI, since a client's Proxy: header can set it
    if environment.contains_key("REQUEST_METHOD") {
        proxies.remove("http");
    }
    for (key, value) in lowercase {
        if value.is_empty() {
            proxies.remove(&key);
        } else {
            proxies.insert(key, value.clone());
        }
    }
    proxies
}

fn fallback_routes(
    proxies: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    warnings: &mut Vec<ProxyWarning>,
) -> BTreeMap<String, ProxyRoute> {
    let netrc = read_netrc(environment, warnings);
    let mut routes = BTreeMap::new();
    for (name, value) in proxies {
        if !matches!(name.as_str(), "http" | "https" | "ws" | "wss") {
            continue;
        }
        match parse_route(name, value, netrc.as_ref()) {
            ProxyRoute::Unsupported { scheme } if scheme == "https" || scheme == "wss" => {
                let ignored = name.clone();
                warnings.push(ProxyWarning::SecureProxyIgnored {
                    name: ignored,
                    scheme,
                });
            }
            route @ (ProxyRoute::Forward(_)
            | ProxyRoute::Unsupported { .. }
            | ProxyRoute::Invalid { .. }) => drop(routes.insert(name.clone(), route)),
        }
    }
    routes
}

fn bypasses(no_proxy: &str, host: &str) -> bool {
    if no_proxy == "*" {
        return true;
    }
    let lowered = host.to_lowercase();
    let host_only = lowered
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(lowered.as_str(), |(prefix, _)| prefix);
    no_proxy
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let name = entry.trim_start_matches('.').to_lowercase();
            let suffix = format!(".{name}");
            // only an IPv6 host loses a tail to the port split, and it has no dots for a suffix
            host_only == name || lowered == name || lowered.ends_with(&suffix)
        })
}

fn parse_route(name: &str, value: &str, netrc: Option<&Netrc>) -> ProxyRoute {
    let invalid = || ProxyRoute::Invalid {
        name: name.to_owned(),
    };
    let Ok(url) = Url::parse(value) else {
        return invalid();
    };
    if url.scheme() != "http" {
        return ProxyRoute::Unsupported {
            scheme: url.scheme().to_owned(),
        };
    }
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return invalid();
    };
    let credentials = if url.username().is_empty() && url.password().is_none() {
        netrc.and_then(|found| netrc_credentials(found, host))
    } else {
        let decoded_username = percent_decode_str(url.username()).decode_utf8();
        let decoded_password = percent_decode_str(url.password().unwrap_or_default()).decode_utf8();
        let (Ok(username), Ok(password)) = (decoded_username, decoded_password) else {
            return invalid();
        };
        Some(ProxyCredentials {
            username: username.into_owned(),
            password: SecretString::from(password.into_owned()),
        })
    };
    ProxyRoute::Forward(Arc::new(ProxyServer {
        host: host.to_owned(),
        port,
        credentials,
    }))
}

fn netrc_credentials(netrc: &Netrc, host: &str) -> Option<ProxyCredentials> {
    let entries = || netrc.machines.iter().rev();
    let entry = entries()
        .find(|machine| machine.name.as_deref() == Some(host))
        .or_else(|| entries().find(|machine| machine.name.is_none()))?;
    let login = entry.login.clone().filter(|login| !login.is_empty());
    let password = entry.password.clone().unwrap_or_default();
    Some(ProxyCredentials {
        username: login.or_else(|| entry.account.clone()).unwrap_or_default(),
        password: SecretString::from(password),
    })
}

fn read_netrc(
    environment: &BTreeMap<String, String>,
    warnings: &mut Vec<ProxyWarning>,
) -> Option<Netrc> {
    let named = environment.get("NETRC");
    let home = environment
        .get("HOME")
        .map(|home| Path::new(home).join(".netrc"));
    let path = named.map(PathBuf::from).or(home)?;
    let parsed = match fs::read_to_string(&path) {
        // aiohttp stays quiet about a missing ~/.netrc, but not about a missing NETRC file
        Err(error) if named.is_none() && error.kind() == ErrorKind::NotFound => return None,
        Err(_) => None,
        Ok(text) => Netrc::parse(text, false).ok(),
    };
    if parsed.is_none() {
        warnings.push(ProxyWarning::NetrcUnusable { path });
    }
    parsed
}

mod tests;

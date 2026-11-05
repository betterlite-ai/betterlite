use std::io;
use std::net::SocketAddr;

use async_trait::async_trait;
use betterlite_config::tls::Verification;
use tokio::net::lookup_host;

use crate::user_url::address::is_blocked_ip;
use crate::user_url::error::{SsrfError, UserUrlError};
use crate::user_url::split::{Parsed, urlparse};

#[async_trait]
pub trait Resolve: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}

// The system's getaddrinfo, through tokio's blocking pool
#[derive(Debug, Clone, Copy)]
pub struct SystemResolver;

#[async_trait]
impl Resolve for SystemResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(lookup_host((host, port)).await?.collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    pub url: String,
    pub host_header: String,
}

#[derive(Debug, Clone)]
pub struct UrlValidator<R> {
    resolver: R,
    allowed_hosts: Vec<String>,
    keep_https: bool,
}

struct Target {
    hostname: String,
    port: Option<u16>,
    effective_port: u16,
    https: bool,
    host_header: String,
}

// An IPv6 host goes between brackets wherever it sits next to a port
fn bracketed(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn format_host_header(hostname: &str, port: u16, default_port: u16) -> String {
    let host = bracketed(hostname);
    if port == default_port {
        host
    } else {
        format!("{host}:{port}")
    }
}

impl Target {
    fn from_parsed(parsed: &Parsed) -> Result<Self, UserUrlError> {
        if parsed.scheme != "http" && parsed.scheme != "https" {
            return Err(UserUrlError::Ssrf(SsrfError::SchemeNotAllowed {
                scheme: parsed.scheme.clone(),
            }));
        }
        let hostname = parsed.hostname().ok_or(SsrfError::NoHostname)?;
        let port = parsed.port()?;
        let https = parsed.scheme == "https";
        let default_port = if https { 443 } else { 80 };
        let effective_port = port.unwrap_or(default_port);
        Ok(Self {
            host_header: format_host_header(&hostname, effective_port, default_port),
            hostname,
            port,
            effective_port,
            https,
        })
    }
}

impl<R> UrlValidator<R>
where
    R: Resolve,
{
    // An https URL keeps its hostname while TLS verification is on, since the certificate binds the connection to it
    pub fn new(resolver: R, allowed_hosts: &[String], verification: &Verification) -> Self {
        Self {
            resolver,
            allowed_hosts: allowed_hosts
                .iter()
                .filter(|entry| !entry.is_empty())
                .map(|entry| entry.to_lowercase().trim_end_matches('.').to_owned())
                .collect(),
            keep_https: *verification != Verification::Disabled,
        }
    }

    fn is_allowlisted(&self, hostname: &str, port: u16) -> bool {
        let shown = bracketed(hostname.to_lowercase().trim_end_matches('.'));
        let with_port = format!("{shown}:{port}");
        self.allowed_hosts
            .iter()
            .any(|entry| *entry == shown || *entry == with_port)
    }

    // Resolves the host and returns the first address, after every address passed the blocklist unless the host is allowlisted
    async fn pinned_address(&self, target: &Target) -> Result<SocketAddr, SsrfError> {
        let addresses = self
            .resolver
            .resolve(&target.hostname, target.effective_port)
            .await
            .map_err(|source| SsrfError::DnsFailed {
                host: target.hostname.clone(),
                source,
            })?;
        let first = addresses.first().ok_or_else(|| SsrfError::NoAddresses {
            host: target.hostname.clone(),
        })?;
        let allowlisted = self.is_allowlisted(&target.hostname, target.effective_port);
        let blocked = addresses.iter().find(|entry| is_blocked_ip(entry.ip()));
        match blocked {
            Some(entry) if !allowlisted => Err(SsrfError::BlockedAddress {
                address: entry.ip(),
            }),
            _ => Ok(*first),
        }
    }

    // LiteLLM validate_url: the URL to connect to and the Host header to send
    pub async fn validate_url(&self, url: &str) -> Result<Validated, UserUrlError> {
        let parsed = urlparse(url, "")?;
        let target = Target::from_parsed(&parsed)?;
        let first = self.pinned_address(&target).await?;
        if target.https && self.keep_https {
            return Ok(Validated {
                url: url.to_owned(),
                host_header: target.host_header,
            });
        }
        let ip_host = bracketed(&first.ip().to_string());
        let netloc = target.port.map_or_else(
            || ip_host.clone(),
            |explicit| format!("{ip_host}:{explicit}"),
        );
        let rewritten = Parsed {
            netloc,
            fragment: String::new(),
            ..parsed
        };
        Ok(Validated {
            url: rewritten.unparse(),
            host_header: target.host_header,
        })
    }
}

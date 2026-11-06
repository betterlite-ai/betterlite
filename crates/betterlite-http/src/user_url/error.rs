use std::io;
use std::net::IpAddr;

use thiserror::Error;

use crate::proxy::transport_error::TransportError;

#[derive(Debug, Error)]
pub enum SsrfError {
    #[error("URL scheme '{scheme}' is not allowed")]
    SchemeNotAllowed { scheme: String },
    #[error("URL has no hostname")]
    NoHostname,
    #[error("DNS resolution failed for '{host}': {source}")]
    DnsFailed {
        host: String,
        #[source]
        source: io::Error,
    },
    #[error("No addresses found for '{host}'")]
    NoAddresses { host: String },
    #[error(
        "URL targets a blocked address ({address}). If this is a legitimate internal service, add the host to `user_url_allowed_hosts` in litellm_settings."
    )]
    BlockedAddress { address: IpAddr },
    #[error("Redirect response has no Location header")]
    NoLocation,
    #[error("Too many redirects")]
    TooManyRedirects,
}

// The ValueError texts of Python's urllib.parse
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UrlParseError {
    #[error("Invalid IPv6 URL")]
    InvalidIpv6Url,
    #[error("IPvFuture address is invalid")]
    InvalidIpvFuture,
    #[error("An IPv4 address cannot be in brackets")]
    Ipv4InBrackets,
    #[error("'{host}' does not appear to be an IPv4 or IPv6 address")]
    NotAnIpAddress { host: String },
    #[error("netloc '{netloc}' contains invalid characters under NFKC normalization")]
    NetlocNfkc { netloc: String },
    #[error("Port could not be cast to integer value as '{port}'")]
    PortNotInteger { port: String },
    #[error("Port out of range 0-65535")]
    PortOutOfRange,
}

// The InvalidURL texts of httpx, which is not a ValueError
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvalidUrl {
    #[error("URL too long")]
    TooLong,
    #[error("Invalid non-printable ASCII character in URL, {character} at position {position}.")]
    NonPrintable { character: String, position: usize },
    #[error("Invalid IPv4 address: '{host}'")]
    Ipv4 { host: String },
    #[error("Invalid IPv6 address: '[{host}]'")]
    Ipv6 { host: String },
    #[error("Invalid IDNA hostname: '{host}'")]
    Idna { host: String },
    #[error("Invalid port: '{port}'")]
    Port { port: String },
    #[error("For absolute URLs, path must be empty or begin with '/'")]
    PathNotAbsolute,
    #[error("Relative URLs cannot have a path starting with '//'")]
    RelativeDoubleSlash,
    #[error("Relative URLs cannot have a path starting with ':'")]
    RelativeColon,
}

#[derive(Debug, Error)]
pub enum UserUrlError {
    #[error(transparent)]
    Ssrf(#[from] SsrfError),
    #[error(transparent)]
    Parse(#[from] UrlParseError),
    #[error(transparent)]
    InvalidUrl(#[from] InvalidUrl),
    #[error("Invalid URL in location header: {0}.")]
    LocationHeader(#[source] InvalidUrl),
    #[error("the URL {url} cannot be requested")]
    Unrequestable {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("Exceeded maximum allowed redirects.")]
    TooManyRedirects,
    #[error(transparent)]
    Transport(#[from] TransportError),
}

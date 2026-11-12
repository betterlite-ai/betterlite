use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::database_url_policy::url_codec::{UrlSplitError, parse_qsl, urlsplit};
use crate::database_urls::settings::DEFAULT_POSTGRES_PORT;

// CPython quote(safe=""): the RFC 3986 unreserved characters stay literal and a space becomes %20
pub(crate) const QUOTE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IAMEndpoint {
    pub host: String,
    pub port: String,
    pub user: String,
    pub name: String,
    pub schema: Option<String>,
}

impl IAMEndpoint {
    // the token arrives already percent-encoded from both providers, so re-encoding it would double-escape the password
    #[must_use]
    pub fn build_url(&self, token: &str) -> String {
        let mut url = format!(
            "postgresql://{}:{token}@{}:{}/{}",
            normalize_quote(&self.user),
            self.host,
            self.port,
            normalize_quote(&self.name)
        );
        if let Some(schema) = self.schema.as_deref().filter(|schema| !schema.is_empty()) {
            url.push_str("?schema=");
            url.push_str(&normalize_quote(schema));
        }
        url
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    #[error("Cannot parse IAM endpoint from URL: missing host or username")]
    MissingHostOrUsername,
    #[error("Cannot parse IAM endpoint from URL: missing database name")]
    MissingDatabaseName,
    #[error("Cannot parse IAM endpoint from URL: the URL is not readable")]
    Url {
        #[source]
        source: UrlSplitError,
    },
    #[error("Cannot parse IAM endpoint from URL: the port is not a valid port number")]
    Port,
}

// DATABASE_USER may already be percent-encoded, so decode once before quoting: the round trip leaves an encoded value untouched
fn normalize_quote(value: &str) -> String {
    let decoded = percent_decode_str(value).decode_utf8_lossy();
    let mut encoded = String::with_capacity(value.len());
    for part in utf8_percent_encode(&decoded, QUOTE_SET) {
        encoded.push_str(part);
    }
    encoded
}

// urllib's _hostinfo: a bracketed host ends at ']', any other host at the first ':', and an empty port counts as absent
fn split_hostport(hostport: &str) -> (&str, Option<&str>) {
    let (host, tail) = hostport.strip_prefix('[').map_or_else(
        || {
            hostport
                .split_once(':')
                .map_or((hostport, None), |(name, rest)| (name, Some(rest)))
        },
        |bracketed| {
            let (address, after_bracket) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            (address, after_bracket.split_once(':').map(|(_, rest)| rest))
        },
    );
    (host, tail.filter(|text| !text.is_empty()))
}

fn parse_port(port: Option<&str>) -> Result<String, EndpointError> {
    let Some(text) = port else {
        return Ok(DEFAULT_POSTGRES_PORT.to_owned());
    };
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EndpointError::Port);
    }
    let value = text.parse::<u16>().map_err(|_error| EndpointError::Port)?;
    if value == 0 {
        Ok(DEFAULT_POSTGRES_PORT.to_owned())
    } else {
        Ok(value.to_string())
    }
}

pub fn parse_iam_endpoint_from_url(url: &str) -> Result<IAMEndpoint, EndpointError> {
    let parsed = urlsplit(url).map_err(|source| EndpointError::Url { source })?;
    let (userinfo, hostport) = match parsed.netloc.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, parsed.netloc.as_str()),
    };
    let raw_username = userinfo.map(|info| info.split_once(':').map_or(info, |(user, _)| user));
    let (host, port_text) = split_hostport(hostport);
    let Some(present_user) = raw_username.filter(|user| !user.is_empty()) else {
        return Err(EndpointError::MissingHostOrUsername);
    };
    if host.is_empty() {
        return Err(EndpointError::MissingHostOrUsername);
    }
    // urllib.parse.unquote decodes once and leaves a bad escape as a literal, so a malformed escape never fails parsing
    let name = percent_decode_str(parsed.path.trim_start_matches('/'))
        .decode_utf8_lossy()
        .into_owned();
    if name.is_empty() {
        return Err(EndpointError::MissingDatabaseName);
    }
    let port = parse_port(port_text)?;
    let schema = parse_qsl(&parsed.query, false)
        .into_iter()
        .find(|(key, _)| key == "schema")
        .map(|(_, value)| value);
    // urllib lowercases only the address before an IPv6 zone id
    let hostname = match host.split_once('%') {
        Some((address, zone)) => format!("{}%{zone}", address.to_lowercase()),
        None => host.to_lowercase(),
    };
    Ok(IAMEndpoint {
        host: hostname,
        port,
        user: percent_decode_str(present_user)
            .decode_utf8_lossy()
            .into_owned(),
        name,
        schema,
    })
}

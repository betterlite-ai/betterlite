use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::database_url_policy::decoder::{DbError, QueryParamValue, decode_database_url};
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
        source: DbError,
    },
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

pub fn parse_iam_endpoint_from_url(url: &str) -> Result<IAMEndpoint, EndpointError> {
    let parsed =
        decode_database_url(url.as_bytes()).map_err(|source| EndpointError::Url { source })?;
    let Some(user) = parsed.user.as_deref().filter(|user| !user.is_empty()) else {
        return Err(EndpointError::MissingHostOrUsername);
    };
    if parsed.host.is_empty() {
        return Err(EndpointError::MissingHostOrUsername);
    }
    if parsed.database.is_empty() {
        return Err(EndpointError::MissingDatabaseName);
    }
    // urllib's parser reports an explicit zero port as absent, so the default covers both
    let port = match parsed.port {
        Some(port) if port != 0 => port.to_string(),
        Some(_) | None => DEFAULT_POSTGRES_PORT.to_owned(),
    };
    let schema = parsed.query.iter().find_map(|(key, entry)| match entry {
        QueryParamValue::Plain(text) if key == "schema" && !text.is_empty() => Some(text.clone()),
        QueryParamValue::Plain(_) | QueryParamValue::Secret(_) => None,
    });
    Ok(IAMEndpoint {
        host: parsed.host.to_lowercase(),
        port,
        user: user.to_owned(),
        name: parsed.database,
        schema,
    })
}

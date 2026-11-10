use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

const QUOTE_PLUS_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~')
    .remove(b' ');

pub(crate) struct Endpoint<'text> {
    pub(crate) user: &'text str,
    pub(crate) password: Option<&'text SecretString>,
    pub(crate) host: &'text str,
    pub(crate) port: &'text str,
    pub(crate) name: &'text str,
    pub(crate) schema: Option<&'text str>,
}

// Mirrors CPython's quote_plus: alphanumerics and _.-~ stay literal, a space becomes '+', every other UTF-8 byte is escaped
#[must_use]
pub(crate) fn quote_plus(text: &str) -> String {
    utf8_percent_encode(text, QUOTE_PLUS_SET)
        .to_string()
        .replace(' ', "+")
}

#[must_use]
pub(crate) fn password_url(endpoint: &Endpoint<'_>) -> String {
    let user = quote_plus(endpoint.user);
    let name = quote_plus(endpoint.name);
    let mut url = endpoint
        .password
        .filter(|password| !password.expose_secret().is_empty())
        .map_or_else(
            || {
                format!(
                    "postgresql://{user}@{}:{}/{name}",
                    endpoint.host, endpoint.port
                )
            },
            |password| {
                format!(
                    "postgresql://{user}:{}@{}:{}/{name}",
                    quote_plus(password.expose_secret()),
                    endpoint.host,
                    endpoint.port
                )
            },
        );
    if let Some(schema) = endpoint.schema.filter(|schema| !schema.is_empty()) {
        url.push_str("?schema=");
        url.push_str(schema);
    }
    url
}

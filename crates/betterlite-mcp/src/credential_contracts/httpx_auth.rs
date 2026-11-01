mod tests;

use http::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::errors::{CredError, CredResult};

pub type HeaderMap = http::HeaderMap;

pub trait Auth {
    fn apply(&self, headers: &mut HeaderMap);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpAuth;

impl Auth for NoOpAuth {
    fn apply(&self, _headers: &mut HeaderMap) {}
}

#[derive(Debug, Clone)]
pub struct StaticHeaderAuth {
    header_name: HeaderName,
    header_value: HeaderValue,
}

impl StaticHeaderAuth {
    // Checking the name and value here, once, means apply() can insert them without a fallible path that would otherwise let a bad header silently vanish.
    pub fn new(header_name: &str, header_value: &SecretString) -> CredResult<Self> {
        let name = HeaderName::from_bytes(header_name.as_bytes()).map_err(|_invalid| {
            CredError::Misconfigured(format!("invalid upstream header name: '{header_name}'"))
        })?;
        let mut value = HeaderValue::from_bytes(header_value.expose_secret().as_bytes()).map_err(
            |_invalid| {
                CredError::Misconfigured(format!(
                    "invalid upstream header value for '{header_name}'"
                ))
            },
        )?;
        value.set_sensitive(true);
        Ok(Self {
            header_name: name,
            header_value: value,
        })
    }
}

impl Auth for StaticHeaderAuth {
    // HeaderName normalizes case and insert() replaces any prior value, so a request that already carries the header never ends up with two.
    fn apply(&self, headers: &mut HeaderMap) {
        headers.insert(self.header_name.clone(), self.header_value.clone());
    }
}

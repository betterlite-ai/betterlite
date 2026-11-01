mod tests;

use http::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

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
    pub header_name: String,
    pub header_value: SecretString,
}

impl Auth for StaticHeaderAuth {
    // HeaderName normalizes case and insert() replaces any prior value, so a request that already carries the header never ends up with two.
    fn apply(&self, headers: &mut HeaderMap) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(self.header_name.as_bytes()),
            HeaderValue::from_str(self.header_value.expose_secret()),
        ) {
            headers.insert(name, value);
        }
    }
}

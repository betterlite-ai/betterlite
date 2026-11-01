mod tests;

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

pub type HeaderMap = BTreeMap<String, String>;

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
    fn apply(&self, headers: &mut HeaderMap) {
        headers.insert(
            self.header_name.clone(),
            self.header_value.expose_secret().to_owned(),
        );
    }
}

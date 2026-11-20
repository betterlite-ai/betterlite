use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

// The caller's own credential for the request; a non-empty one wins over everything the gateway holds.
#[derive(Debug, Clone)]
pub enum McpAuthOverride {
    Text(SecretString),
    Headers(BTreeMap<String, SecretString>),
}

impl McpAuthOverride {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(value) => value.expose_secret().is_empty(),
            Self::Headers(headers) => headers.is_empty(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum McpAuthValue {
    Text(SecretString),
    Headers(BTreeMap<String, SecretString>),
}

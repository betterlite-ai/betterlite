use std::cmp::Ordering;

use secrecy::{ExposeSecret as _, SecretString};

use super::IAM_AUTH_KEY;

// the credential itself keys the cache, and the wrapper keeps it typed while the map compares it
#[derive(Clone, Debug)]
pub struct CredentialKey(pub SecretString);

impl PartialEq for CredentialKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.expose_secret() == other.0.expose_secret()
    }
}

impl Eq for CredentialKey {}

impl PartialOrd for CredentialKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CredentialKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.expose_secret().cmp(other.0.expose_secret())
    }
}

#[must_use]
pub fn vertex_instance_key(credentials: Option<&SecretString>) -> SecretString {
    let Some(given) = credentials else {
        return SecretString::from(IAM_AUTH_KEY);
    };
    given.clone()
}

mod tests;

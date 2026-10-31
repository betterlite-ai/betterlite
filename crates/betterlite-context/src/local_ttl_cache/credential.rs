use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct Credential {
    pub token: SecretString,
    pub headers: BTreeMap<String, SecretString>,
}

#[derive(Clone, Debug)]
pub struct CredentialHandle(pub Arc<Credential>);

impl PartialEq for CredentialHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl CredentialHandle {
    pub(super) fn size_charge(&self) -> usize {
        let token_size = self.0.token.expose_secret().len();
        self.0
            .headers
            .iter()
            .fold(token_size, |size, (key, value)| {
                size.saturating_add(key.len())
                    .saturating_add(value.expose_secret().len())
            })
    }
}

#[derive(Debug)]
pub struct CredentialIdentity {
    pub authority: String,
    pub principal: String,
}

impl CredentialIdentity {
    #[must_use]
    pub fn cache_key(&self, resource: &str) -> String {
        let authority_len = self.authority.len();
        let principal_len = self.principal.len();
        format!(
            "{authority_len}:{}{principal_len}:{}{resource}",
            self.authority, self.principal
        )
    }
}

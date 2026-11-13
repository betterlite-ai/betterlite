use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone, Debug)]
pub struct StoredCredential {
    pub base_url: String,
    pub key: Option<SecretString>,
}

pub trait GatewayKeyStore {
    // The expected URL reaches the store so a vault-backed store can answer a foreign origin from the file alone
    fn stored_credential(&self, expected_base_url: Option<&str>) -> Option<StoredCredential>;
}

#[must_use]
pub fn get_litellm_gateway_api_key(
    expected_base_url: Option<&str>,
    store: &dyn GatewayKeyStore,
) -> Option<String> {
    let credential = store.stored_credential(expected_base_url)?;
    if let Some(expected) = expected_base_url
        && credential.base_url != expected.trim_end_matches('/')
    {
        return None;
    }
    credential.key.map(|key| key.expose_secret().to_owned())
}

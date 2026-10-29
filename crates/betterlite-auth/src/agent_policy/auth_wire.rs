use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

// Serde's library adapter creates this private, non-Debug value only during explicit serialization.
#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

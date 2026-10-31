use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SecretText(
    #[serde(
        serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
    )]
    pub SecretString,
);

// The adapter exposes this non-Debug value only at the explicit serialization boundary.
#[derive(Serialize)]
#[serde(transparent)]
struct ExposedSecret(String);

impl From<&SecretString> for ExposedSecret {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

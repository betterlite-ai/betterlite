use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

#[expect(
    clippy::disallowed_types,
    reason = "bounded passthrough members retain JSON outside the modeled field type"
)]
pub type ExtraValue = serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Member<T> {
    Known(T),
    Other(ExtraValue),
}

impl<T> Member<T> {
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Other(_) => None,
        }
    }
}

#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct ExposedSecret(String);

impl From<&Member<SecretString>> for Member<ExposedSecret> {
    fn from(value: &Member<SecretString>) -> Self {
        match value {
            Member::Known(token) => Self::Known(ExposedSecret(token.expose_secret().to_owned())),
            Member::Other(extra) => Self::Other(extra.clone()),
        }
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
pub(super) struct PresentMember<T>(Member<T>);

impl<T> From<PresentMember<T>> for Option<Member<T>> {
    fn from(value: PresentMember<T>) -> Self {
        Some(value.0)
    }
}

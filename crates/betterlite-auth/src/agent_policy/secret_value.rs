use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug, Formatter};

#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SecretValue(String);

impl Debug for SecretValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(REDACTED)")
    }
}

impl SecretValue {
    pub(super) const fn expose(&self) -> &str {
        match self {
            Self(value) => value.as_str(),
        }
    }
}

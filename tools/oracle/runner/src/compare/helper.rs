use betterlite_oracle_helper_api::error::HelperError;
use secrecy::{ExposeSecret as _, SecretString};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HelperWarning {
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HelperOutput {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HelperError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<HelperWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<HelperEffect>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HelperEffect {
    pub kind: String,
    pub fields: ManagementEffectFields,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementEffectFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", skip_serializing)]
    pub credential: Option<SecretString>,
}

impl ManagementEffectFields {
    #[must_use]
    pub fn same_secret(&self, other: &Self) -> bool {
        match (&self.credential, &other.credential) {
            (Some(left), Some(right)) => left.expose_secret() == right.expose_secret(),
            (None, None) => true,
            _ => false,
        }
    }
}

impl PartialEq for ManagementEffectFields {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.same_secret(other)
    }
}

use secrecy::{ExposeSecret as _, SecretString};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperWarning {
    pub(crate) subject: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperOutput {
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HelperError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<HelperWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) effects: Vec<HelperEffect>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperEffect {
    pub(crate) kind: String,
    pub(crate) fields: ManagementEffectFields,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagementEffectFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", skip_serializing)]
    pub(crate) credential: Option<SecretString>,
}

impl ManagementEffectFields {
    pub(crate) fn same_secret(&self, other: &Self) -> bool {
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

#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "Only the crate's test module names ErrorCode, so the non-test build has no use for the re-export."
    )
)]
pub(crate) use betterlite_oracle_helper_api::error::{ErrorCode, HelperError};

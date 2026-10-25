use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize as _, Deserializer as WireDecoder};

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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        skip_serializing,
        deserialize_with = "parse_secret"
    )]
    pub(crate) credential: Option<SecretString>,
}

fn parse_secret<'de, D>(decoder: D) -> Result<Option<SecretString>, D::Error>
where
    D: WireDecoder<'de>,
{
    let value = Option::<String>::deserialize(decoder)?;
    Ok(value.map(SecretString::from))
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

type OptionalString = Option<Option<String>>;
type OptionalCode = Option<Option<ErrorCode>>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct HelperError {
    pub(crate) class: String,
    pub(crate) status: Option<u16>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) error_type: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) param: OptionalString,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub(crate) code: OptionalCode,
    pub(crate) message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum ErrorCode {
    Text(String),
    Number(serde_json::Number),
}

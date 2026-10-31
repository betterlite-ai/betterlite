use serde::{Deserialize, Serialize};

use super::{ResourceRowsError, convert, json_value_is_set};
use crate::budget_permission_rows::JsonData;
use crate::identity_rows::Payload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialBase {
    pub credential_name: String,
    pub credential_info: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialItem {
    #[serde(flatten)]
    pub base: CredentialBase,
    pub credential_values: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCredentialItemFields {
    #[serde(flatten)]
    pub base: CredentialBase,
    #[serde(default)]
    pub credential_values: Option<Payload>,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct CreateCredentialItem {
    #[serde(flatten)]
    pub fields: CreateCredentialItemFields,
}

impl TryFrom<JsonData> for CreateCredentialItem {
    type Error = ResourceRowsError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let JsonData::Object(object) = &value else {
            return Err(Self::Error::NotAnObject);
        };
        let values_set = object
            .get("credential_values")
            .is_some_and(json_value_is_set);
        let model_id_set = object.get("model_id").is_some_and(json_value_is_set);
        if !values_set && !model_id_set {
            return Err(Self::Error::CredentialValueRequired);
        }
        Ok(Self {
            fields: convert(&value)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCredentialItem {
    pub credential_name: String,
    pub credential_info: Payload,
    #[serde(default)]
    pub credential_values: Option<Payload>,
    #[serde(default)]
    pub model_id: Option<String>,
}

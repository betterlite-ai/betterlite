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
pub struct CredentialItemFields {
    #[serde(flatten)]
    pub base: CredentialBase,
    pub credential_values: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct CredentialItem {
    #[serde(flatten)]
    pub fields: CredentialItemFields,
}

impl TryFrom<JsonData> for CredentialItem {
    type Error = ResourceRowsError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        check_value_shape(&value)?;
        Ok(Self {
            fields: convert(&value)?,
        })
    }
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

pub(crate) fn check_create_guard(value: &JsonData) -> Result<(), ResourceRowsError> {
    let JsonData::Object(object) = value else {
        return Err(ResourceRowsError::NotAnObject);
    };
    let has_value = ["credential_values", "model_id"]
        .into_iter()
        .any(|field| object.get(field).is_some_and(json_value_is_set));
    if has_value {
        Ok(())
    } else {
        Err(ResourceRowsError::CredentialValueRequired)
    }
}

impl TryFrom<JsonData> for CreateCredentialItem {
    type Error = ResourceRowsError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        check_create_guard(&value)?;
        check_value_shape(&value)?;
        Ok(Self {
            fields: convert(&value)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCredentialItemFields {
    pub credential_name: String,
    pub credential_info: Payload,
    #[serde(default)]
    pub credential_values: Option<Payload>,
    #[serde(default)]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct UpdateCredentialItem {
    #[serde(flatten)]
    pub fields: UpdateCredentialItemFields,
}

impl TryFrom<JsonData> for UpdateCredentialItem {
    type Error = ResourceRowsError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        check_value_shape(&value)?;
        Ok(Self {
            fields: convert(&value)?,
        })
    }
}

// credential_values holds a secret, so a shape mismatch here never carries the raw value.
pub(crate) fn check_value_shape(value: &JsonData) -> Result<(), ResourceRowsError> {
    let JsonData::Object(object) = value else {
        return Ok(());
    };
    let Some(field_value) = object.get("credential_values") else {
        return Ok(());
    };
    if matches!(field_value, JsonData::Null | JsonData::Object(_)) {
        return Ok(());
    }
    let text = serde_json::to_string(field_value).map_err(ResourceRowsError::Json)?;
    match serde_json::from_str::<Payload>(&text) {
        Ok(_) => Ok(()),
        Err(error) => Err(ResourceRowsError::FieldShape {
            field: "credential_values",
            category: error.classify(),
            line: error.line(),
            column: error.column(),
        }),
    }
}

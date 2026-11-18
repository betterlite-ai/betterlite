use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

use super::TeamContractError;
use super::shared::decode_request;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMetadataFieldSchema {
    pub key: String,
    pub label: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamMetadataFieldInput {
    key: String,
    label: Option<String>,
}

impl TeamMetadataFieldSchema {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let input: TeamMetadataFieldInput = decode_request(value)?;
        if input.key.is_empty() {
            return Err(TeamContractError::Validation {
                message: "String should have at least 1 character".to_owned(),
            });
        }
        Ok(Self {
            key: input.key,
            label: input.label,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMetadataSchemaResponse {
    pub fields: Vec<TeamMetadataFieldSchema>,
}

#[derive(Deserialize)]
struct TeamMetadataSchemaInput {
    fields: Vec<JsonData>,
}

impl TeamMetadataSchemaResponse {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let input: TeamMetadataSchemaInput = decode_request(value)?;
        let fields = input
            .fields
            .iter()
            .map(TeamMetadataFieldSchema::from_value)
            .collect::<Result<Vec<TeamMetadataFieldSchema>, TeamContractError>>()?;
        Ok(Self { fields })
    }
}

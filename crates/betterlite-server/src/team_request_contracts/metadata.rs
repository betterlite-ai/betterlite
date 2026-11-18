use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

use super::TeamContractError;
use super::shared::decode_request;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamMetadataFieldSchema {
    pub key: String,
    pub label: Option<String>,
}

impl TeamMetadataFieldSchema {
    pub fn from_value(value: &JsonData) -> Result<Self, TeamContractError> {
        let field: Self = decode_request(value)?;
        if field.key.is_empty() {
            return Err(TeamContractError::Validation {
                message: "String should have at least 1 character".to_owned(),
            });
        }
        Ok(field)
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
        Ok(Self {
            fields: field_schemas(&input.fields)?,
        })
    }
}

fn field_schemas(items: &[JsonData]) -> Result<Vec<TeamMetadataFieldSchema>, TeamContractError> {
    items
        .iter()
        .map(TeamMetadataFieldSchema::from_value)
        .collect()
}

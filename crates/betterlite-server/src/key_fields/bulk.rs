use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use super::broadcast::KeyUpdateFields;
use super::decode::{TypedValue, decode_group, to_object};
use super::error::DecodeError;
use super::values::{Amount, LiteLLMObjectPermissionBase};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BulkUpdateKeyRequestItem {
    pub key: String,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub max_budget: Option<Amount>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BulkUpdateKeyRequest {
    pub keys: Vec<BulkUpdateKeyRequestItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SuccessfulKeyUpdate {
    pub key: String,
    pub key_info: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FailedKeyUpdate {
    pub key: String,
    #[serde(default)]
    pub key_info: Option<Payload>,
    pub failed_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BulkUpdateKeyResponse {
    pub total_requested: i64,
    pub successful_updates: Vec<SuccessfulKeyUpdate>,
    pub failed_updates: Vec<FailedKeyUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BulkUpdateTeamKeysRequest {
    pub team_id: String,
    #[serde(default)]
    pub key_ids: Option<Vec<String>>,
    #[serde(default)]
    pub all_keys_in_team: bool,
    pub update_fields: KeyUpdateFields,
}

impl BulkUpdateTeamKeysRequest {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let request: Self = decode_group(&value.object()?)?;
        request.update_fields.validate()?;
        request.validate_selection()?;
        Ok(request)
    }

    // Nonempty key_ids and all_keys_in_team are exclusive, and one of them is required.
    fn validate_selection(&self) -> Result<(), DecodeError> {
        let has_key_ids = self.key_ids.as_ref().is_some_and(|ids| !ids.is_empty());
        if has_key_ids && self.all_keys_in_team {
            return Err(DecodeError::SelectionConflict);
        }
        if !has_key_ids && !self.all_keys_in_team {
            return Err(DecodeError::SelectionMissing);
        }
        Ok(())
    }

    pub fn resolved_object(&self) -> Result<BTreeMap<String, JsonData>, DecodeError> {
        let mut fields = to_object(self)?;
        let updates = self.update_fields.project()?.resolved;
        fields.insert("update_fields".to_owned(), JsonData::Object(updates));
        Ok(fields)
    }
}

use betterlite_db::identity_rows::Payload;

use super::base::KeyRequestBase;
use super::decode::{KeyProjection, TypedValue, blank_to_null, decode_group, project, to_object};
use super::defaults::{BASE_DEFAULTS, GENERATE_DEFAULTS, KEY_DEFAULTS, REGENERATE_DEFAULTS};
use super::error::DecodeError;
use super::request::GenerateFields;
use super::values::Amount;

// The key, new_key and new_master_key fields are secrets, so this record derives no Debug, PartialEq or Eq.
nullable_record! {
    (Clone, serde::Serialize, serde::Deserialize)
    pub struct RegenerateFields {
        duration: String,
        spend: Amount,
        metadata: Payload,
        grace_period: String,
    } secret { key, new_key, new_master_key }
}

#[derive(Clone)]
pub struct RegenerateKeyRequest {
    pub base: KeyRequestBase,
    pub generate: GenerateFields,
    pub regenerate: RegenerateFields,
}

impl RegenerateKeyRequest {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        blank_to_null(
            &mut fields,
            &["max_budget", "team_id", "organization_id", "project_id"],
        );
        Ok(Self {
            base: KeyRequestBase::decode(&fields)?,
            generate: decode_group(&fields)?,
            regenerate: decode_group(&fields)?,
        })
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let base = to_object(&self.base.base)?;
        let key = to_object(&self.base.key_fields)?;
        let generate = to_object(&self.generate)?;
        let regenerate = to_object(&self.regenerate)?;
        project(
            &[
                BASE_DEFAULTS,
                KEY_DEFAULTS,
                GENERATE_DEFAULTS,
                REGENERATE_DEFAULTS,
            ],
            &[&base, &key, &generate, &regenerate],
            self.base.object_permission(),
        )
    }
}

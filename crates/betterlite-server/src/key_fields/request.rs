use super::base::KeyRequestBase;
use super::decode::{KeyProjection, TypedValue, blank_to_null, decode_group, project, to_object};
use super::defaults::{BASE_DEFAULTS, GENERATE_DEFAULTS, KEY_DEFAULTS};
use super::error::DecodeError;
use super::values::{Amount, LiteLLMKeyType};

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    pub struct GenerateFields {
        soft_budget: Amount,
        send_invite_email: bool,
        key_type: LiteLLMKeyType,
        auto_rotate: bool,
        rotation_interval: String,
        organization_id: String,
        project_id: String,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateKeyRequest {
    pub base: KeyRequestBase,
    pub generate: GenerateFields,
}

impl GenerateKeyRequest {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        blank_to_null(
            &mut fields,
            &["max_budget", "team_id", "organization_id", "project_id"],
        );
        Ok(Self {
            base: KeyRequestBase::decode(&fields)?,
            generate: decode_group(&fields)?,
        })
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let base = to_object(&self.base.base)?;
        let key = to_object(&self.base.key_fields)?;
        let generate = to_object(&self.generate)?;
        project(
            &[BASE_DEFAULTS, KEY_DEFAULTS, GENERATE_DEFAULTS],
            &[&base, &key, &generate],
            self.base.object_permission(),
        )
    }
}

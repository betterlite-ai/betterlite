use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::Payload;

use super::base::KeyRequestBase;
use super::decode::{
    KeyProjection, TypedValue, blank_to_null, decode_group, drop_blank, project, to_object,
};
use super::defaults::{BASE_DEFAULTS, KEY_DEFAULTS, UPDATE_DEFAULTS};
use super::error::DecodeError;
use super::values::Amount;

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    pub struct UpdateFields {
        duration: String,
        spend: Amount,
        soft_budget: Amount,
        metadata: Payload,
        temp_budget_increase: Amount,
        temp_budget_expiry: Timestamp,
        auto_rotate: bool,
        rotation_interval: String,
        organization_id: String,
        project_id: String,
    }
}

#[derive(Clone)]
pub struct UpdateKeyRequest {
    pub base: KeyRequestBase,
    pub update: UpdateFields,
}

impl UpdateKeyRequest {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        drop_blank(&mut fields, "team_id");
        blank_to_null(&mut fields, &["max_budget", "organization_id"]);
        let request = Self {
            base: KeyRequestBase::decode(&fields)?,
            update: decode_group(&fields)?,
        };
        request.validate()?;
        Ok(request)
    }

    // The two after validators run in declaration order: the temporary pair, then the key identifier.
    const fn validate(&self) -> Result<(), DecodeError> {
        let increase = matches!(self.update.temp_budget_increase, Some(Some(_)));
        let expiry = matches!(self.update.temp_budget_expiry, Some(Some(_)));
        if increase != expiry {
            return Err(DecodeError::TemporaryBudgetPair);
        }
        let key = matches!(self.base.key_fields.key, Some(Some(_)));
        let alias = matches!(self.base.base.key_alias, Some(Some(_)));
        if !key && !alias {
            return Err(DecodeError::MissingKeyIdentifier);
        }
        Ok(())
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let base = to_object(&self.base.base)?;
        let key = to_object(&self.base.key_fields)?;
        let update = to_object(&self.update)?;
        project(
            &[BASE_DEFAULTS, KEY_DEFAULTS, UPDATE_DEFAULTS],
            &[&base, &key, &update],
            self.base.object_permission(),
        )
    }
}

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::Payload;

use super::decode::{KeyProjection, TypedValue, decode_group, project, to_object};
use super::defaults::BROADCAST_DEFAULTS;
use super::error::DecodeError;
use super::values::{Amount, LimitType};

// The bulk-broadcast allowlist, alphabetical so the projection's key order is its own.
pub const BROADCAST_FIELDS: [&str; 17] = [
    "budget_duration",
    "budget_id",
    "budget_limits",
    "duration",
    "max_budget",
    "max_parallel_requests",
    "metadata",
    "model_max_budget",
    "model_rpm_limit",
    "model_tpm_limit",
    "rpm_limit",
    "rpm_limit_type",
    "tags",
    "temp_budget_expiry",
    "temp_budget_increase",
    "tpm_limit",
    "tpm_limit_type",
];

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    #[serde(deny_unknown_fields)]
    pub struct KeyUpdateFields {
        max_budget: Amount,
        budget_id: String,
        budget_duration: String,
        budget_limits: Vec<JsonData>,
        model_max_budget: Payload,
        tpm_limit: i64,
        rpm_limit: i64,
        model_tpm_limit: Payload,
        model_rpm_limit: Payload,
        max_parallel_requests: i64,
        rpm_limit_type: LimitType,
        tpm_limit_type: LimitType,
        temp_budget_increase: Amount,
        temp_budget_expiry: Timestamp,
        duration: String,
        tags: Vec<String>,
        metadata: Payload,
    }
}

impl KeyUpdateFields {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let fields: Self = decode_group(&value.object()?)?;
        fields.validate()?;
        Ok(fields)
    }

    // The after validators run in declaration order: the temporary pair, then the empty-patch check.
    pub(crate) fn validate(&self) -> Result<(), DecodeError> {
        let increase = matches!(self.temp_budget_increase, Some(Some(_)));
        let expiry = matches!(self.temp_budget_expiry, Some(Some(_)));
        if increase != expiry {
            return Err(DecodeError::TemporaryBudgetPair);
        }
        if to_object(self)?.is_empty() {
            return Err(DecodeError::EmptyPatch);
        }
        Ok(())
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let fields = to_object(self)?;
        let groups = [&fields];
        project(&[BROADCAST_DEFAULTS], &groups, None)
    }
}

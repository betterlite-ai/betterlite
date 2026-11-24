use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::key_fields::base::KeyRequestBase;
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::shared::hash_token_if_needed;
use crate::key_fields::update::UpdateKeyRequest;
use crate::key_fields::values::{Amount, LimitType};
use crate::management_guards::amounts::Amount as GuardAmount;

// the GenerateKeyRequest or UpdateKeyRequest fields the checks read, with null and absent both None
#[derive(Debug, Clone)]
pub struct KeyLimitRequest {
    pub max_budget: Option<Decimal>,
    pub soft_budget: Option<Decimal>,
    pub max_parallel_requests: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub tpd_limit: Option<i64>,
    pub duration: Option<String>,
    pub budget_duration: Option<String>,
    pub models: Option<Vec<JsonData>>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub model_rpm_limit: Option<BTreeMap<String, JsonData>>,
    pub model_tpm_limit: Option<BTreeMap<String, JsonData>>,
    pub rpm_limit_type: Option<LimitType>,
    pub tpm_limit_type: Option<LimitType>,
    // _hash_token_if_needed(data.key), set only for an UpdateKeyRequest that carries a key
    pub update_key_hash: Option<String>,
}

impl From<&GenerateKeyRequest> for KeyLimitRequest {
    fn from(request: &GenerateKeyRequest) -> Self {
        Self::read(&request.base, request.generate.soft_budget.flatten(), None)
    }
}

impl From<&UpdateKeyRequest> for KeyLimitRequest {
    fn from(request: &UpdateKeyRequest) -> Self {
        let key = request
            .base
            .key_fields
            .key
            .as_ref()
            .and_then(Option::as_ref);
        let hash = key.map(|secret| hash_token_if_needed(secret.expose_secret()));
        Self::read(&request.base, request.update.soft_budget.flatten(), hash)
    }
}

impl KeyLimitRequest {
    fn read(base: &KeyRequestBase, soft_budget: Option<Amount>, key_hash: Option<String>) -> Self {
        let fields = &base.base;
        Self {
            max_budget: fields.max_budget.flatten().map(|amount| amount.0),
            soft_budget: soft_budget.map(|amount| amount.0),
            max_parallel_requests: fields.max_parallel_requests.flatten(),
            tpm_limit: fields.tpm_limit.flatten(),
            rpm_limit: fields.rpm_limit.flatten(),
            tpd_limit: base.key_fields.tpd_limit.flatten(),
            duration: fields.duration.clone().flatten(),
            budget_duration: fields.budget_duration.clone().flatten(),
            models: fields.models.clone().flatten(),
            metadata: fields.metadata.clone().flatten().map(|payload| payload.0),
            model_rpm_limit: fields
                .model_rpm_limit
                .clone()
                .flatten()
                .map(|payload| payload.0),
            model_tpm_limit: fields
                .model_tpm_limit
                .clone()
                .flatten()
                .map(|payload| payload.0),
            rpm_limit_type: base.key_fields.rpm_limit_type.flatten(),
            tpm_limit_type: base.key_fields.tpm_limit_type.flatten(),
            update_key_hash: key_hash,
        }
    }

    // the six _BUDGET_NUMERIC_KEYS in the request model's declaration order
    #[must_use]
    pub fn numeric_values(&self) -> [NumericValue; 6] {
        let count = |number: Option<i64>| number.map(Decimal::from);
        [
            ("max_budget", self.max_budget),
            ("max_parallel_requests", count(self.max_parallel_requests)),
            ("tpm_limit", count(self.tpm_limit)),
            ("rpm_limit", count(self.rpm_limit)),
            ("tpd_limit", count(self.tpd_limit)),
            ("soft_budget", self.soft_budget),
        ]
        .map(|(field, value)| NumericValue {
            field,
            value: value.map(GuardAmount::Finite),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericValue {
    pub field: &'static str,
    pub value: Option<GuardAmount>,
}

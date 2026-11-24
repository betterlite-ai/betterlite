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

// LiteLLM's Union[GenerateKeyRequest, UpdateKeyRequest], read through its getattr calls
#[derive(Clone, Copy)]
pub enum KeyLimitRequest<'request> {
    Generate(&'request GenerateKeyRequest),
    Update(&'request UpdateKeyRequest),
}

impl<'request> KeyLimitRequest<'request> {
    const fn base(&self) -> &'request KeyRequestBase {
        match *self {
            Self::Generate(request) => &request.base,
            Self::Update(request) => &request.base,
        }
    }

    #[must_use]
    pub fn max_budget(&self) -> Option<Decimal> {
        self.base()
            .base
            .max_budget
            .as_ref()
            .and_then(Option::as_ref)
            .map(|amount| amount.0)
    }

    #[must_use]
    pub fn soft_budget(&self) -> Option<Amount> {
        let field = match *self {
            Self::Generate(request) => &request.generate.soft_budget,
            Self::Update(request) => &request.update.soft_budget,
        };
        field.as_ref().and_then(Option::as_ref).copied()
    }

    #[must_use]
    pub fn max_parallel_requests(&self) -> Option<i64> {
        self.base().base.max_parallel_requests.flatten()
    }

    #[must_use]
    pub fn tpm_limit(&self) -> Option<i64> {
        self.base().base.tpm_limit.flatten()
    }

    #[must_use]
    pub fn rpm_limit(&self) -> Option<i64> {
        self.base().base.rpm_limit.flatten()
    }

    #[must_use]
    pub fn tpd_limit(&self) -> Option<i64> {
        self.base().key_fields.tpd_limit.flatten()
    }

    #[must_use]
    pub fn duration(&self) -> Option<&str> {
        self.base()
            .base
            .duration
            .as_ref()
            .and_then(Option::as_ref)
            .map(String::as_str)
    }

    #[must_use]
    pub fn budget_duration(&self) -> Option<&str> {
        self.base()
            .base
            .budget_duration
            .as_ref()
            .and_then(Option::as_ref)
            .map(String::as_str)
    }

    #[must_use]
    pub fn models(&self) -> Option<&[JsonData]> {
        self.base()
            .base
            .models
            .as_ref()
            .and_then(Option::as_ref)
            .map(Vec::as_slice)
    }

    #[must_use]
    pub fn metadata(&self) -> Option<&BTreeMap<String, JsonData>> {
        self.base()
            .base
            .metadata
            .as_ref()
            .and_then(Option::as_ref)
            .map(|payload| &payload.0)
    }

    #[must_use]
    pub fn model_rpm_limit(&self) -> Option<&BTreeMap<String, JsonData>> {
        self.base()
            .base
            .model_rpm_limit
            .as_ref()
            .and_then(Option::as_ref)
            .map(|payload| &payload.0)
    }

    #[must_use]
    pub fn model_tpm_limit(&self) -> Option<&BTreeMap<String, JsonData>> {
        self.base()
            .base
            .model_tpm_limit
            .as_ref()
            .and_then(Option::as_ref)
            .map(|payload| &payload.0)
    }

    #[must_use]
    pub fn rpm_limit_type(&self) -> Option<LimitType> {
        self.base().key_fields.rpm_limit_type.flatten()
    }

    #[must_use]
    pub fn tpm_limit_type(&self) -> Option<LimitType> {
        self.base().key_fields.tpm_limit_type.flatten()
    }

    // _hash_token_if_needed(data.key) only runs for an UpdateKeyRequest that carries a key
    #[must_use]
    pub fn update_key_hash(&self) -> Option<String> {
        match self {
            Self::Update(request) => request
                .base
                .key_fields
                .key
                .as_ref()?
                .as_ref()
                .map(|key| hash_token_if_needed(key.expose_secret())),
            Self::Generate(_) => None,
        }
    }

    // the six _BUDGET_NUMERIC_KEYS in the request model's declaration order
    #[must_use]
    pub fn numeric_values(&self) -> [NumericValue; 6] {
        [
            money("max_budget", self.max_budget()),
            count("max_parallel_requests", self.max_parallel_requests()),
            count("tpm_limit", self.tpm_limit()),
            count("rpm_limit", self.rpm_limit()),
            count("tpd_limit", self.tpd_limit()),
            money("soft_budget", self.soft_budget().map(|amount| amount.0)),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericValue {
    pub field: &'static str,
    pub value: Option<GuardAmount>,
}

fn money(field: &'static str, value: Option<Decimal>) -> NumericValue {
    NumericValue {
        field,
        value: value.map(GuardAmount::Finite),
    }
}

fn count(field: &'static str, value: Option<i64>) -> NumericValue {
    NumericValue {
        field,
        value: value.map(|number| GuardAmount::Finite(Decimal::from(number))),
    }
}

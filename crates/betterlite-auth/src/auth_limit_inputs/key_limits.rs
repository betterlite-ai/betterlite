use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde_json::Number;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::deployment::{DeploymentLimit, DeploymentLimitCatalog, deployment_default_limit};
use super::error::AuthLimitInputsError;

pub type ModelLimits = BTreeMap<String, i64>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelRateLimitKey {
    Rpm,
    Tpm,
    Itpm,
    Otpm,
}

impl ModelRateLimitKey {
    pub(super) const fn field(self) -> &'static str {
        match self {
            Self::Rpm => "model_rpm_limit",
            Self::Tpm => "model_tpm_limit",
            Self::Itpm => "model_itpm_limit",
            Self::Otpm => "model_otpm_limit",
        }
    }

    // the source's own-limit check treats every rate limit key other than model_rpm_limit as the tpm budget
    const fn budget_field(self) -> &'static str {
        match self {
            Self::Rpm => "rpm_limit",
            Self::Tpm | Self::Itpm | Self::Otpm => "tpm_limit",
        }
    }
}

#[must_use]
pub fn get_key_own_model_rate_limit(
    auth: &UserAPIKeyAuth,
    rate_limit_key: ModelRateLimitKey,
) -> Option<ModelLimits> {
    let metadata = &auth.view.token.metadata.0;
    if !metadata.is_empty() {
        match metadata.get(rate_limit_key.field()) {
            None | Some(JsonData::Null) => {}
            Some(value) => {
                if let Some(limits) = integer_limits(value).filter(|limits| !limits.is_empty()) {
                    return Some(limits);
                }
            }
        }
    }
    let model_max_budget = &auth.view.token.model_max_budget.0;
    if model_max_budget.is_empty() {
        return None;
    }
    model_max_budget_limits(model_max_budget, rate_limit_key.budget_field())
}

pub fn get_key_model_rpm_limit(
    auth: &UserAPIKeyAuth,
    model_name: Option<&str>,
    catalog: Option<&dyn DeploymentLimitCatalog>,
) -> Result<Option<ModelLimits>, AuthLimitInputsError> {
    model_limit_with_deployment_default(auth, model_name, catalog, DeploymentLimit::Rpm)
}

pub fn get_key_model_tpm_limit(
    auth: &UserAPIKeyAuth,
    model_name: Option<&str>,
    catalog: Option<&dyn DeploymentLimitCatalog>,
) -> Result<Option<ModelLimits>, AuthLimitInputsError> {
    model_limit_with_deployment_default(auth, model_name, catalog, DeploymentLimit::Tpm)
}

fn model_limit_with_deployment_default(
    auth: &UserAPIKeyAuth,
    model_name: Option<&str>,
    catalog: Option<&dyn DeploymentLimitCatalog>,
    limit: DeploymentLimit,
) -> Result<Option<ModelLimits>, AuthLimitInputsError> {
    let rate_limit_key = match limit {
        DeploymentLimit::Rpm => ModelRateLimitKey::Rpm,
        DeploymentLimit::Tpm => ModelRateLimitKey::Tpm,
    };
    if let Some(limits) = get_key_own_model_rate_limit(auth, rate_limit_key) {
        return Ok(Some(limits));
    }
    if let Some(limits) = team_model_limit(auth, rate_limit_key) {
        return Ok(Some(limits));
    }
    let Some(requested) = model_name else {
        return Ok(None);
    };
    let Some(default_limit) = deployment_default_limit(catalog, requested, limit)? else {
        return Ok(None);
    };
    Ok(Some(ModelLimits::from([(
        requested.to_owned(),
        default_limit,
    )])))
}

fn team_model_limit(
    auth: &UserAPIKeyAuth,
    rate_limit_key: ModelRateLimitKey,
) -> Option<ModelLimits> {
    let payload = auth.view.team_metadata.as_ref()?;
    let metadata = &payload.0;
    if metadata.is_empty() {
        return None;
    }
    match metadata.get(rate_limit_key.field()) {
        None | Some(JsonData::Null) => None,
        Some(value) => integer_limits(value),
    }
}

fn model_max_budget_limits(
    budgets: &BTreeMap<String, JsonData>,
    budget_field: &str,
) -> Option<ModelLimits> {
    let mut limits = ModelLimits::new();
    for (model, budget) in budgets {
        let JsonData::Object(entry) = budget else {
            continue;
        };
        let Some(value) = entry.get(budget_field) else {
            continue;
        };
        if matches!(value, JsonData::Null) {
            continue;
        }
        let JsonData::Number(number) = value else {
            return None;
        };
        let Some(integer) = json_integer(number) else {
            return None;
        };
        limits.insert(model.clone(), integer);
    }
    if limits.is_empty() {
        None
    } else {
        Some(limits)
    }
}

// a stored model limit map is dict[str, int]; anything else is not a limit the limiter can compare
pub(super) fn integer_limits(value: &JsonData) -> Option<ModelLimits> {
    let JsonData::Object(fields) = value else {
        return None;
    };
    let mut limits = ModelLimits::new();
    for (model, limit) in fields {
        let JsonData::Number(number) = limit else {
            return None;
        };
        let Some(integer) = json_integer(number) else {
            return None;
        };
        limits.insert(model.clone(), integer);
    }
    Some(limits)
}

fn json_integer(number: &Number) -> Option<i64> {
    number
        .as_i64()
        .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
}

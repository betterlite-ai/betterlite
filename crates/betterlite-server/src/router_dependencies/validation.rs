use std::collections::BTreeSet;

use betterlite_models::model_registration::JsonObject;
use betterlite_router::complexity_config::keys::COMPLEXITY_ROUTER_CONFIG_KEYS;

use super::error::{ComplexityPlacementError, StrategyRouterWriteError};
use super::{
    AUTO_ROUTER_MODEL_PREFIX, STRATEGY_ROUTER_PARAM_FIELDS, classify_strategy_router_model,
    required_fields,
};

// a setting written one level too high configures nothing and reaches the provider as an unknown param
pub fn validate_complexity_router_config_placement(
    litellm_params: Option<&JsonObject>,
) -> Result<(), ComplexityPlacementError> {
    let Some(litellm_params) = litellm_params else {
        return Ok(());
    };
    let mut misplaced: Vec<&str> = COMPLEXITY_ROUTER_CONFIG_KEYS
        .iter()
        .copied()
        .filter(|name| litellm_params.contains_key(*name))
        .collect();
    if misplaced.is_empty() {
        return Ok(());
    }
    misplaced.sort_unstable();
    Err(ComplexityPlacementError {
        settings: misplaced.join(", "),
    })
}

pub fn validate_strategy_router_model_write(
    model: &str,
    present_fields: &BTreeSet<String>,
) -> Result<(), StrategyRouterWriteError> {
    let Some(kind) = classify_strategy_router_model(model) else {
        let mut offending: Vec<&str> = STRATEGY_ROUTER_PARAM_FIELDS
            .iter()
            .copied()
            .filter(|name| present_fields.contains(*name))
            .collect();
        if offending.is_empty() {
            return Ok(());
        }
        offending.sort_unstable();
        return Err(StrategyRouterWriteError::UnexpectedSettings {
            model: model.to_owned(),
            settings: offending.join(", "),
        });
    };
    let remainder = model
        .strip_prefix(AUTO_ROUTER_MODEL_PREFIX)
        .unwrap_or_default();
    if remainder.starts_with(AUTO_ROUTER_MODEL_PREFIX) {
        return Err(StrategyRouterWriteError::RepeatedPrefix {
            model: model.to_owned(),
            remainder: remainder.to_owned(),
        });
    }
    if remainder.is_empty() {
        return Err(StrategyRouterWriteError::MissingRouterName {
            model: model.to_owned(),
        });
    }
    let mut missing = Vec::new();
    for &group in required_fields(kind) {
        if group.iter().all(|name| !present_fields.contains(*name)) {
            missing.push(group.join(" or "));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(StrategyRouterWriteError::MissingSettings {
        model: model.to_owned(),
        kind,
        requirements: missing.join("; "),
    })
}

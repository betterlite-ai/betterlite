use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::tiered_pricing::Tier;
use rust_decimal::Decimal;

use super::error::{ModelInfoError, ReservationCostError};
use super::rates::{float_of, int_of};
use super::{ModelInfoSource, RouterModelInfo};
use crate::tiering::{TieringError, pricing_tiers};

pub(crate) struct CostInfo {
    pub(crate) image_mode: bool,
    pub(crate) max_input_tokens: Option<i64>,
    pub(crate) max_output_tokens: Option<i64>,
    pub(crate) input_cost_per_token: Option<Decimal>,
    pub(crate) output_cost_per_token: Option<Decimal>,
    pub(crate) output_cost_per_reasoning_token: Option<Decimal>,
    pub(crate) input_cost_per_image: Option<Decimal>,
    pub(crate) output_cost_per_image: Option<Decimal>,
    pub(crate) tiered_pricing: Option<Vec<Tier>>,
}

impl CostInfo {
    fn from_json(info: &JsonObject) -> Result<Self, TieringError> {
        Ok(Self {
            image_mode: matches!(info.get("mode"), Some(Json::Text(mode)) if mode == "image_generation" || mode == "image_edit"),
            max_input_tokens: int_of(info.get("max_input_tokens")),
            max_output_tokens: int_of(info.get("max_output_tokens")),
            input_cost_per_token: float_of(info.get("input_cost_per_token")),
            output_cost_per_token: float_of(info.get("output_cost_per_token")),
            output_cost_per_reasoning_token: float_of(info.get("output_cost_per_reasoning_token")),
            input_cost_per_image: float_of(info.get("input_cost_per_image")),
            output_cost_per_image: float_of(info.get("output_cost_per_image")),
            tiered_pricing: pricing_tiers(info.get("tiered_pricing"))?,
        })
    }
}

pub(crate) fn load(
    model: &str,
    router: Option<&dyn RouterModelInfo>,
    source: &dyn ModelInfoSource,
) -> Result<Vec<CostInfo>, ReservationCostError> {
    let candidates = match candidate_objects(model, router, source) {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::debug!(error = %error, "unable to load model cost info for budget reservation");
            return Ok(Vec::new());
        }
    };
    let mut infos = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        infos.push(CostInfo::from_json(candidate)?);
    }
    Ok(infos)
}

// Reservation runs before routing, so every pricing shape an eligible deployment may bill is a candidate.
fn candidate_objects(
    model: &str,
    router: Option<&dyn RouterModelInfo>,
    source: &dyn ModelInfoSource,
) -> Result<Vec<JsonObject>, ModelInfoError> {
    let Some(base) = base_cost_info(model, router, source)? else {
        return Ok(Vec::new());
    };
    let tables = deployment_tiered_tables(model, router)?;
    let mut candidates = Vec::with_capacity(tables.len().saturating_add(1));
    candidates.push(base.clone());
    for table in tables {
        let mut merged = base.clone();
        merged.insert("tiered_pricing".to_owned(), table);
        candidates.push(merged);
    }
    Ok(candidates)
}

fn base_cost_info(
    model: &str,
    router: Option<&dyn RouterModelInfo>,
    source: &dyn ModelInfoSource,
) -> Result<Option<JsonObject>, ModelInfoError> {
    if let Some(router) = router
        && let Some(group_info) = router.cached_model_group_info(model)?
    {
        return Ok(Some(group_info));
    }
    source.get_model_info(model)
}

fn deployment_tiered_tables(
    model: &str,
    router: Option<&dyn RouterModelInfo>,
) -> Result<Vec<Json>, ModelInfoError> {
    let Some(router) = router else {
        return Ok(Vec::new());
    };
    let mut tables = Vec::new();
    for deployment in router.get_model_list(model)? {
        let (Some(model_id), Some(backend_model)) = (deployment.model_id, deployment.backend_model)
        else {
            continue;
        };
        let Some(info) = router.cached_deployment_model_info(&model_id, &backend_model)? else {
            continue;
        };
        if let Some(table @ Json::List(tiers)) = info.get("tiered_pricing")
            && !tiers.is_empty()
        {
            tables.push(table.clone());
        }
    }
    Ok(tables)
}

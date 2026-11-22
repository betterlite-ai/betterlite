use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::shadow_eval_validation::error::ShadowEvalRequestError;
use crate::shadow_eval_validation::request::fields::{
    RawRequest, bounded_list, budget, deduplicated, duration_days, percentage, validation,
};

mod fields;

pub const DEFAULT_SHADOW_EVAL_JUDGE_MODEL: &str = "anthropic/claude-sonnet-5";
// Sample-count ceiling on every new job: a zero-cost error loop never consumes spend budget, so it must terminate on count.
pub const SHADOW_EVAL_TURN_VALVE: u32 = 10_000;
pub const SHADOW_EVAL_MAX_ROUTERS: usize = 4;
pub const SHADOW_EVAL_MAX_TARGETS: usize = 100;
pub const MAX_TURNS_MESSAGE: &str =
    "max_turns was replaced by max_budget, the per-target USD cap on the eval's own spend";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowEvalDirection {
    #[default]
    Forward,
    Reverse,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartShadowEvalRequest {
    pub api_key_ids: Vec<String>,
    pub team_ids: Vec<String>,
    pub user_ids: Vec<String>,
    pub models: Vec<String>,
    pub router_names: Vec<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub shadow_percentage: f64,
    pub judge_model: String,
    pub duration_days: u32,
    pub max_budget: Decimal,
}

// The request model's own outside boundary: 1 MiB of bytes and 64 levels of JSON before any field is read.
pub fn parse_shadow_eval_request(
    bytes: &[u8],
) -> Result<StartShadowEvalRequest, ShadowEvalRequestError> {
    let loaded = safe_json_loads(bytes).map_err(ShadowEvalRequestError::Bounded)?;
    let document =
        serde_json::from_value::<JsonData>(loaded).map_err(ShadowEvalRequestError::Decode)?;
    let JsonData::Object(object) = document else {
        return Err(ShadowEvalRequestError::NotAnObject);
    };
    // the retired budget is rejected before any field is read, as the model's mode="before" validator runs
    if object.contains_key("max_turns") {
        return Err(validation("max_turns", MAX_TURNS_MESSAGE.to_owned()));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let raw =
        serde_path_to_error::deserialize::<_, RawRequest>(&mut deserializer).map_err(|error| {
            ShadowEvalRequestError::Field {
                path: error.path().to_string(),
                source: error.into_inner(),
            }
        })?;
    fields_of(raw)
}

fn fields_of(raw: RawRequest) -> Result<StartShadowEvalRequest, ShadowEvalRequestError> {
    let api_key_ids = bounded_list(raw.api_key_ids, "api_key_ids", SHADOW_EVAL_MAX_TARGETS)?;
    let team_ids = bounded_list(raw.team_ids, "team_ids", SHADOW_EVAL_MAX_TARGETS)?;
    let user_ids = bounded_list(raw.user_ids, "user_ids", SHADOW_EVAL_MAX_TARGETS)?;
    let models = bounded_list(raw.models, "models", SHADOW_EVAL_MAX_TARGETS)?;
    if models.iter().any(|name| name.trim().is_empty()) {
        return Err(validation(
            "models",
            "models must be non-empty model group names".to_owned(),
        ));
    }
    let declared_routers = bounded_list(raw.router_names, "router_names", SHADOW_EVAL_MAX_ROUTERS)?;
    let request = StartShadowEvalRequest {
        api_key_ids,
        team_ids,
        user_ids,
        models,
        router_names: declared_routers,
        direction: raw.direction.unwrap_or_default(),
        baseline_model: raw.baseline_model,
        shadow_percentage: percentage(raw.shadow_percentage)?,
        judge_model: raw
            .judge_model
            .unwrap_or_else(|| DEFAULT_SHADOW_EVAL_JUDGE_MODEL.to_owned()),
        duration_days: duration_days(raw.duration_days)?,
        max_budget: budget(raw.max_budget)?,
    };
    validate(request, raw.router_name)
}

fn validate(
    request: StartShadowEvalRequest,
    router_name: Option<String>,
) -> Result<StartShadowEvalRequest, ShadowEvalRequestError> {
    target_count(&request)?;
    if !request.models.is_empty() && request.direction == ShadowEvalDirection::Reverse {
        return Err(validation(
            "models",
            "models is only meaningful for a forward job; a reverse job samples its own router's traffic"
                .to_owned(),
        ));
    }
    let baseline = request.baseline_model.is_some();
    if request.direction == ShadowEvalDirection::Reverse && !baseline {
        return Err(validation(
            "baseline_model",
            "baseline_model is required when direction is 'reverse'".to_owned(),
        ));
    }
    if request.direction == ShadowEvalDirection::Forward && baseline {
        return Err(validation(
            "baseline_model",
            "baseline_model is only meaningful when direction is 'reverse'".to_owned(),
        ));
    }
    router_set(request, router_name.as_deref())
}

fn target_count(request: &StartShadowEvalRequest) -> Result<(), ShadowEvalRequestError> {
    let total = request
        .api_key_ids
        .len()
        .saturating_add(request.team_ids.len())
        .saturating_add(request.user_ids.len());
    if total < 1 {
        return Err(validation(
            "api_key_ids",
            "at least one target is required: pass api_key_ids, team_ids, or user_ids".to_owned(),
        ));
    }
    if total > SHADOW_EVAL_MAX_TARGETS {
        return Err(validation(
            "api_key_ids",
            "at most 100 targets per job across api_key_ids, team_ids, and user_ids".to_owned(),
        ));
    }
    Ok(())
}

// _resolve_router_set: whichever spelling arrived, the request leaves with one deduplicated set.
fn router_set(
    mut request: StartShadowEvalRequest,
    router_name: Option<&str>,
) -> Result<StartShadowEvalRequest, ShadowEvalRequestError> {
    let has_single = router_name.is_some();
    let has_plural = !request.router_names.is_empty();
    if has_single == has_plural {
        return Err(validation(
            "router_names",
            "provide exactly one of router_name or router_names".to_owned(),
        ));
    }
    let declared = router_name.map_or_else(
        || request.router_names.clone(),
        |name| vec![name.to_owned()],
    );
    let routers = deduplicated(declared);
    if routers.iter().any(|name| name.trim().is_empty()) {
        return Err(validation(
            "router_names",
            "router names must be non-empty strings".to_owned(),
        ));
    }
    if routers.len() > 1 && request.direction == ShadowEvalDirection::Reverse {
        return Err(validation(
            "router_names",
            "a reverse job evaluates one router against baseline_model; pass a single router"
                .to_owned(),
        ));
    }
    request.router_names = routers;
    Ok(request)
}

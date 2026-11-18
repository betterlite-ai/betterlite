use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

use crate::deployment_identity::spec::DeploymentSpec;

use super::costs::{Catalog, ModelCosts, usable};
use super::error::CostRoutingError;
use super::window::WindowUsage;

// model_cost.get("input_cost_per_token", 5.0), applied to each side of the sum the selector ranks by.
const DEFAULT_COST_PER_TOKEN: Decimal = Decimal::from_parts(5, 0, 0, false, 0);

const INPUT_COST: &str = "input_cost_per_token";
const OUTPUT_COST: &str = "output_cost_per_token";
const TPM: &str = "tpm";
const RPM: &str = "rpm";
const MODEL: &str = "model";

// One selection: the group whose map is read, the healthy pool, and the request's own token estimate.
pub struct Selection<'request> {
    pub group: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub input_tokens: i64,
}

#[derive(Clone, Copy)]
struct Limits {
    tpm: Option<Decimal>,
    rpm: Option<Decimal>,
}

// async_get_available_deployments: rank the deployments the map's order and this minute's limits leave eligible.
pub(super) fn select(
    costs: &dyn ModelCosts,
    request: &Selection<'_>,
    order: &[String],
    used: &[WindowUsage],
) -> Result<Option<usize>, CostRoutingError> {
    let mut candidates: Vec<(usize, Decimal)> = Vec::new();
    for (position, deployment_id) in order.iter().enumerate() {
        let Some(index) = request
            .deployments
            .iter()
            .rposition(|deployment| deployment.id() == Some(deployment_id.as_str()))
        else {
            continue;
        };
        let Some(deployment) = request.deployments.get(index) else {
            continue;
        };
        let name = deployment.id().unwrap_or_default();
        let limits = limits(deployment, name)?;
        let cost = cost(costs, deployment, name)?;
        let usage = used.get(position).copied().unwrap_or(WindowUsage::ZERO);
        if over_limits(usage, limits, request.input_tokens) {
            continue;
        }
        candidates.push((index, cost));
    }
    // sorted() is stable, so equal costs keep the map's first-observation order
    candidates.sort_by_key(|candidate| candidate.1);
    Ok(candidates.first().map(|(index, _cost)| *index))
}

fn limits(deployment: &DeploymentSpec, name: &str) -> Result<Limits, CostRoutingError> {
    Ok(Limits {
        tpm: limit(deployment, name, TPM)?,
        rpm: limit(deployment, name, RPM)?,
    })
}

// The pinned `or` chain: a top-level deployment limit, then litellm_params, then model_info, then infinity.
fn limit(
    deployment: &DeploymentSpec,
    name: &str,
    field: &'static str,
) -> Result<Option<Decimal>, CostRoutingError> {
    let chosen = [
        deployment.extra.get(field),
        deployment.litellm_params.get(field),
        deployment.model_info.get(field),
    ]
    .into_iter()
    .flatten()
    .find(|value| usable(value));
    chosen.map(|value| amount(value, field, name)).transpose()
}

// The pinned metric: a litellm_params override while it is truthy, else the model cost map entry, else both defaults.
fn cost(
    costs: &dyn ModelCosts,
    deployment: &DeploymentSpec,
    name: &str,
) -> Result<Decimal, CostRoutingError> {
    let entry = match deployment.litellm_params.get(MODEL) {
        Some(Json::Text(model)) => costs.entry(model),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let input = rate(
        deployment.litellm_params.get(INPUT_COST),
        entry,
        INPUT_COST,
        name,
    )?;
    let output = rate(
        deployment.litellm_params.get(OUTPUT_COST),
        entry,
        OUTPUT_COST,
        name,
    )?;
    Ok(input.saturating_add(output))
}

fn rate(
    custom: Option<&Json>,
    entry: Option<Catalog<'_>>,
    field: &'static str,
    name: &str,
) -> Result<Decimal, CostRoutingError> {
    let configured = custom.filter(|value| usable(value));
    let catalog = entry.and_then(|catalog| catalog.price(field));
    match configured.or(catalog) {
        None | Some(Json::Null) => Ok(DEFAULT_COST_PER_TOKEN),
        Some(value) => amount(value, field, name),
    }
}

fn amount(value: &Json, field: &'static str, name: &str) -> Result<Decimal, CostRoutingError> {
    let Json::Number(number) = value else {
        return Err(CostRoutingError::NonNumeric {
            deployment: name.to_owned(),
            field,
        });
    };
    parse_decimal_lexeme(&number.to_string()).map_err(|_source| CostRoutingError::NonNumeric {
        deployment: name.to_owned(),
        field,
    })
}

// item_tpm + input_tokens > tpm or item_rpm + 1 > rpm, with a missing limit standing in for float("inf").
fn over_limits(usage: WindowUsage, limits: Limits, input_tokens: i64) -> bool {
    let tokens = Decimal::from(usage.tokens).saturating_add(Decimal::from(input_tokens));
    let requests = Decimal::from(usage.requests).saturating_add(Decimal::ONE);
    limits.tpm.is_some_and(|limit| tokens > limit)
        || limits.rpm.is_some_and(|limit| requests > limit)
}

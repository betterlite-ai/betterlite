use betterlite_models::decisions::Json;
use betterlite_router::adaptive_classification::minimum_quality::minimum_quality_tier;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::adaptive_selection::costs::normalized_cost;
use betterlite_router::adaptive_selection::error::AdaptiveSelectionError;
use betterlite_router::adaptive_selection::pre_routing::PreRoutingRequest;
use betterlite_router::bandit_math::sampler::{QualitySampler, ScriptedSampler};
use betterlite_router::bandit_math::{BanditCell, Candidate, apply_delta, pick_best};
use rust_decimal::Decimal;

use super::wire::{
    CellOutput, Check, CostCheckInput, CostScoreOutput, DeltaInput, Fields, Payload, RoutingOutput,
    SnapshotOutput,
};
use super::{cost_map, decimal, number, sample_values, service, weights};
use crate::router_probes::error::OperationError;

pub(super) fn fields_for(payload: &Payload) -> Result<Fields, OperationError> {
    let mut fields = Fields {
        cells: None,
        selected: None,
        cost_scores: None,
        snapshot: None,
        routing: None,
    };
    match payload.check {
        Check::ApplyDelta => fields.cells = Some(updated_cells(&payload.deltas)?),
        Check::NormalizedCost => fields.cost_scores = Some(cost_scores(&payload.cost_checks)?),
        Check::Pick => fields.selected = Some(selected_candidate(payload)?),
        Check::PickModel => fields.selected = Some(selected_model(payload)?),
        Check::PreRouting => fields.routing = Some(planned_route(payload)?),
        Check::Snapshot => {
            fields.snapshot = Some(SnapshotOutput::from(&service(payload)?.state_snapshot()));
        }
    }
    Ok(fields)
}

fn updated_cells(deltas: &[DeltaInput]) -> Result<Vec<CellOutput>, OperationError> {
    let mut cells = Vec::with_capacity(deltas.len());
    for input in deltas {
        let cell = BanditCell::new(number(&input.alpha, "alpha")?, number(&input.beta, "beta")?)
            .map_err(AdaptiveSelectionError::Bandit)?;
        let updated = apply_delta(
            cell,
            number(&input.delta_alpha, "delta_alpha")?,
            number(&input.delta_beta, "delta_beta")?,
        )
        .map_err(AdaptiveSelectionError::Bandit)?;
        cells.push(CellOutput {
            request_type: input.request_type,
            model: input.model.clone(),
            alpha: format!("{:.6}", updated.alpha),
            beta: format!("{:.6}", updated.beta),
            samples: updated.total_samples(),
        });
    }
    Ok(cells)
}

fn cost_scores(checks: &[CostCheckInput]) -> Result<Vec<CostScoreOutput>, OperationError> {
    let all_amounts = checks
        .iter()
        .map(|check| {
            check
                .all_costs
                .iter()
                .map(|text| decimal(text, "all_costs"))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    checks
        .iter()
        .zip(&all_amounts)
        .map(|(check, amounts)| {
            Ok(CostScoreOutput {
                model: check.model.clone(),
                cost_score: format!(
                    "{:.6}",
                    normalized_cost(decimal(&check.amount, "amount")?, amounts)?
                ),
            })
        })
        .collect()
}

fn selected_candidate(payload: &Payload) -> Result<String, OperationError> {
    let weights = weights(payload)?;
    let costs = cost_map(payload)?;
    let all_costs = payload
        .cells
        .iter()
        .map(|cell| costs.get(&cell.model).copied().unwrap_or(Decimal::ZERO))
        .collect::<Vec<_>>();
    let mut candidates = Vec::with_capacity(payload.cells.len());
    for (cell, cost) in payload.cells.iter().zip(&all_costs) {
        let bandit_cell =
            BanditCell::new(number(&cell.alpha, "alpha")?, number(&cell.beta, "beta")?)
                .map_err(AdaptiveSelectionError::Bandit)?;
        candidates.push(Candidate {
            model: &cell.model,
            cell: bandit_cell,
            economy_score: normalized_cost(*cost, &all_costs)?,
        });
    }
    let mut sampler = QualitySampler::Scripted(Box::new(ScriptedSampler {
        samples: sample_values(payload)?,
        position: 0,
    }));
    Ok(pick_best(&candidates, weights, &mut sampler).map_err(AdaptiveSelectionError::Bandit)?)
}

fn selected_model(payload: &Payload) -> Result<String, OperationError> {
    let mut selection = service(payload)?;
    let request_type = payload.request_type.unwrap_or(RequestType::General);
    Ok(selection.pick_model(request_type, payload.min_quality_tier)?)
}

fn planned_route(payload: &Payload) -> Result<RoutingOutput, OperationError> {
    let mut selection = service(payload)?;
    let mut metadata = payload
        .metadata_present
        .then(|| payload.metadata.clone().unwrap_or(Json::Null));
    let response = selection.pre_route(PreRoutingRequest {
        messages: payload.messages.as_deref(),
        headers: payload.headers.as_ref(),
        metadata: &mut metadata,
    })?;
    let request_type = response.routing_decision.request_type;
    Ok(RoutingOutput {
        model: response.model,
        messages: response.messages.map(|messages| messages.to_vec()),
        routing_decision: response.routing_decision,
        metadata: metadata.ok_or_else(|| OperationError::InvalidRequest {
            message: "the adaptive pre-routing hook left no metadata".to_owned(),
        })?,
        request_type,
        min_quality_tier: minimum_quality_tier(payload.headers.as_ref(), payload.metadata.as_ref()),
    })
}

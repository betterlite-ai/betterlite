use betterlite_models::decisions::Json;
use betterlite_router::adaptive_classification::minimum_quality::minimum_quality_tier;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::adaptive_selection::costs::normalized_cost;
use betterlite_router::adaptive_selection::pre_routing::PreRoutingRequest;
use betterlite_router::bandit_math::sampler::{QualitySampler, ScriptedSampler};
use betterlite_router::bandit_math::{BanditCell, Candidate, apply_delta, pick_best};
use rust_decimal::Decimal;

use super::wire::{
    CellOutput, CostOutput, CostScoreOutput, Fields, Payload, RoutingOutput, SnapshotCellOutput,
    SnapshotOutput, WeightsOutput,
};
use super::{
    cost_map, decimal, number, operation_error, sample_values, selection_error, service, weights,
};
use crate::router_probes::error::OperationError;

pub(super) fn apply_delta_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let mut cells = Vec::with_capacity(payload.deltas.len());
    for input in &payload.deltas {
        let cell = BanditCell::new(number(&input.alpha, "alpha")?, number(&input.beta, "beta")?)
            .map_err(operation_error)?;
        let updated = apply_delta(
            cell,
            number(&input.delta_alpha, "delta_alpha")?,
            number(&input.delta_beta, "delta_beta")?,
        )
        .map_err(operation_error)?;
        cells.push(CellOutput {
            request_type: input.request_type,
            model: input.model.clone(),
            alpha: format!("{:.6}", updated.alpha),
            beta: format!("{:.6}", updated.beta),
            samples: updated.total_samples(),
        });
    }
    Ok(Fields {
        cells: Some(cells),
        ..Fields::default()
    })
}

pub(super) fn normalized_cost_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let mut cost_scores = Vec::with_capacity(payload.cost_checks.len());
    for check in &payload.cost_checks {
        let amounts = check
            .all_costs
            .iter()
            .map(|text| decimal(text, "all_costs"))
            .collect::<Result<Vec<_>, _>>()?;
        let cost_score = normalized_cost(decimal(&check.amount, "amount")?, &amounts)
            .map_err(selection_error)?;
        cost_scores.push(CostScoreOutput {
            model: check.model.clone(),
            cost_score: format!("{cost_score:.6}"),
        });
    }
    Ok(Fields {
        cost_scores: Some(cost_scores),
        ..Fields::default()
    })
}

pub(super) fn pick_fields(payload: &Payload) -> Result<Fields, OperationError> {
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
                .map_err(operation_error)?;
        candidates.push(Candidate {
            model: &cell.model,
            cell: bandit_cell,
            cost_score: normalized_cost(*cost, &all_costs).map_err(selection_error)?,
        });
    }
    let samples = sample_values(payload)?;
    let mut sampler = QualitySampler::Scripted(ScriptedSampler::new(samples));
    let selected = pick_best(&candidates, weights, &mut sampler).map_err(operation_error)?;
    Ok(Fields {
        selected: Some(selected),
        ..Fields::default()
    })
}

pub(super) fn pick_model_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let mut selection = service(payload)?;
    let request_type = payload.request_type.unwrap_or(RequestType::General);
    let selected = selection
        .pick_model(request_type, payload.min_quality_tier)
        .map_err(selection_error)?;
    Ok(Fields {
        selected: Some(selected),
        ..Fields::default()
    })
}

pub(super) fn pre_routing_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let mut selection = service(payload)?;
    let mut metadata = if payload.metadata_present {
        Some(payload.metadata.clone().unwrap_or(Json::Null))
    } else {
        None
    };
    let response = selection
        .pre_route(PreRoutingRequest {
            messages: payload.messages.as_deref(),
            headers: payload.headers.as_ref(),
            metadata: &mut metadata,
        })
        .map_err(selection_error)?;
    let request_type = response.routing_decision.request_type;
    let chosen_metadata = metadata.ok_or_else(|| OperationError::InvalidRequest {
        message: "the adaptive pre-routing hook left no metadata".to_owned(),
    })?;
    Ok(Fields {
        routing: Some(RoutingOutput {
            model: response.model,
            messages: response.messages.map(|messages| messages.to_vec()),
            routing_decision: response.routing_decision,
            metadata: chosen_metadata,
            request_type,
            min_quality_tier: minimum_quality_tier(
                payload.headers.as_ref(),
                payload.metadata.as_ref(),
            ),
        }),
        ..Fields::default()
    })
}

pub(super) fn snapshot_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let selection = service(payload)?;
    let snapshot = selection.state_snapshot();
    let cells = snapshot
        .cells
        .iter()
        .map(|cell| SnapshotCellOutput {
            request_type: cell.request_type,
            model: cell.model.clone(),
            alpha: format!("{:.6}", cell.alpha),
            beta: format!("{:.6}", cell.beta),
            samples: cell.samples,
            quality_mean: format!("{:.6}", cell.quality_mean),
        })
        .collect();
    let model_costs = snapshot
        .model_costs
        .iter()
        .map(|(model, amount)| CostOutput {
            model: model.clone(),
            amount: format!("{amount:.6}"),
        })
        .collect();
    Ok(Fields {
        snapshot: Some(SnapshotOutput {
            router_name: snapshot.router_name,
            available_models: snapshot.available_models,
            weights: WeightsOutput {
                quality: format!("{:.6}", snapshot.weights.quality),
                cost: format!("{:.6}", snapshot.weights.economy),
            },
            model_costs,
            cells,
        }),
        ..Fields::default()
    })
}

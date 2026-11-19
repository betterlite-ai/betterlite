use betterlite_logging::logging_state::LoggingState;
use rust_decimal::Decimal;

use super::super::CostRecorder;
use super::super::CostRequest;
use super::super::diagnostics;
use super::super::identity::{self, BillingIdentity, PreparationError};
use super::super::view;
use super::{Outcome, Resolved, ViewRequest};
use crate::cost::context::ProviderInference;
use crate::cost::prepare::ResponseView;

// The cost calculator path: billing identity, the injected calculator call and its outcome.
pub(super) async fn record_calculated<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    cache_hit: Option<bool>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let billing = match resolve_billing(recorder, log, view, request, cache_hit) {
        Ok(found) => found,
        Err(error) => {
            super::preparation_failure(log, &error.to_string());
            return None;
        }
    };
    let calculated = {
        let calculation = cost_request(&billing, log, view);
        recorder.calculator.calculate(&calculation).await
    };
    record_outcome(
        recorder,
        log,
        view,
        request,
        Outcome {
            billing,
            calculated,
        },
    )
}

// The router id falls back first to the response's hidden model_id, then to the logged metadata.
fn resolve_billing<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    cache_hit: Option<bool>,
) -> Result<BillingIdentity, PreparationError> {
    let router = match request.router_model_id.filter(|id| !id.is_empty()) {
        Some(router) => Some(router.to_owned()),
        None => match view::hidden_text(view, "model_id") {
            Some(hidden) => Some(hidden.to_owned()),
            None => identity::metadata::router_model_id(log)?,
        },
    };
    let custom_pricing = recorder.custom_pricing(log, view)?;
    let resolved = Resolved {
        cache_hit,
        custom_pricing,
        router_model_id: router.as_deref(),
    };
    prepare(recorder, log, request, &resolved)
}

fn record_outcome<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    outcome: Outcome,
) -> Option<Decimal> {
    let Outcome {
        billing,
        calculated,
    } = outcome;
    match calculated {
        Ok(amount) => {
            let total = identity::total_with_additional(amount, identity::additional_cost(log));
            let router = billing.router_model_id.clone();
            recorder.record_diagnostic(
                log,
                super::diagnostic_input(view, request, Some(total), false, router),
            );
            Some(total)
        }
        Err(error) => {
            let message = error.to_string();
            let info = diagnostics::failure_information(&message, Some(&billing), &log.call_type);
            log.model_call_details
                .insert(diagnostics::FAILURE_KEY.to_owned(), info);
            let router = billing.router_model_id.clone();
            recorder.record_diagnostic(
                log,
                super::diagnostic_input(view, request, None, true, router),
            );
            None
        }
    }
}

// Every value response_cost_calculator reads, resolved once the response is priced.
fn prepare<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &LoggingState,
    request: &ViewRequest<'_>,
    resolved: &Resolved<'_>,
) -> Result<BillingIdentity, PreparationError> {
    let model = request
        .litellm_model_name
        .filter(|name| !name.is_empty())
        .unwrap_or(&log.model)
        .to_owned();
    let provider = identity::custom_llm_provider(log)?;
    let base = identity::metadata::base_model(log)?;
    let prompt = identity::prompt(log);
    let residency = identity::data_residency(log)?;
    let vertex = identity::vertex_location(log, &model, recorder.vertex, recorder.regions)?;
    let region = identity::mantle_region(log, recorder.mantle)?;
    Ok(BillingIdentity {
        model,
        cache_hit: resolved.cache_hit,
        custom_llm_provider: provider,
        base_model: base,
        custom_pricing: resolved.custom_pricing,
        router_model_id: resolved.router_model_id.map(str::to_owned),
        data_residency: residency,
        vertex_location: vertex,
        region_name: region,
        prompt,
    })
}

fn cost_request<'request>(
    billing: &'request BillingIdentity,
    log: &'request LoggingState,
    view: &'request ResponseView<'request>,
) -> CostRequest<'request> {
    CostRequest {
        model: &billing.model,
        cache_hit: billing.cache_hit,
        custom_llm_provider: billing.custom_llm_provider.as_deref(),
        base_model: billing.base_model.as_deref(),
        call_type: &log.call_type,
        optional_params: &log.optional_params,
        custom_pricing: billing.custom_pricing,
        prompt: &billing.prompt,
        standard_built_in_tools_params: &log.standard_built_in_tools_params,
        router_model_id: billing.router_model_id.as_deref(),
        data_residency: billing.data_residency.as_deref(),
        vertex_location: billing.vertex_location.as_deref(),
        region_name: billing.region_name.as_deref(),
        response: view,
    }
}

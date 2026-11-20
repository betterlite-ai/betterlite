use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::diagnostics::{self, DiagnosticInput};
use super::identity::{self, BillingIdentity, PreparationError};
use super::view::{self, JsonResponseError};
use super::{CostRecorder, CostRequest, PortFailure};
use crate::cost::context::ProviderInference;
use crate::cost::prepare::ResponseView;

// The per-call values every entry point passes, before anything is resolved.
pub(super) struct ViewRequest<'names> {
    pub(super) cache_hit: Option<bool>,
    pub(super) litellm_model_name: Option<&'names str>,
    pub(super) router_model_id: Option<&'names str>,
    pub(super) background: bool,
}

enum HiddenOutcome {
    Absent,
    Found(Decimal),
    Invalid,
}

// The mapping, event and generateContent arms all arrive as this result.
pub(super) async fn record_prepared<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    prepared: Result<Option<ResponseView<'_>>, JsonResponseError>,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    match prepared {
        Ok(Some(parsed)) => record_view(recorder, log, &parsed, request).await,
        Ok(None) => record_view(recorder, log, &ResponseView::Other, request).await,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            None
        }
    }
}

// Ports _generate_content_result_as_model_response: a failed conversion leaves the body as it was.
pub(super) async fn record_generate_content<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    body: &Json,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let converted = match recorder.conversion.convert(body, &log.model) {
        Ok(converted) => converted,
        Err(error) => {
            tracing::debug!(%error, "generate_content response cost normalization failed");
            return record_prepared(recorder, log, view::body_view(body), request).await;
        }
    };
    match view::body_view(&converted) {
        Ok(Some(parsed)) => record_view(recorder, log, &parsed, request).await,
        Ok(None) => record_prepared(recorder, log, view::body_view(body), request).await,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            None
        }
    }
}

pub(super) async fn record_view<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let cache_hit = request.cache_hit.or(identity::stored_cache_hit(log));
    if cache_hit == Some(true) || identity::unbilled(log, request.background) {
        return Some(Decimal::ZERO);
    }
    match hidden_cost(recorder, log, view, request) {
        HiddenOutcome::Absent => {}
        HiddenOutcome::Found(cost) => return Some(cost),
        HiddenOutcome::Invalid => return None,
    }
    record_calculated(recorder, log, view, request, cache_hit).await
}

// Ports the hidden-response-cost branch: it wins over the calculator and skips additional_response_cost.
fn hidden_cost<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
) -> HiddenOutcome
where
    Inference: ProviderInference,
{
    let value = view::typed(view)
        .and_then(|typed| typed.hidden_params)
        .and_then(|params| params.get("response_cost"));
    let cost = match value {
        None | Some(Json::Null) => return HiddenOutcome::Absent,
        Some(value) => match identity::amount(value) {
            Some(cost) => cost,
            None => {
                preparation_failure(log, "the hidden response_cost must be a number");
                return HiddenOutcome::Invalid;
            }
        },
    };
    let router = request
        .router_model_id
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| view::hidden_text(view, "model_id").map(str::to_owned));
    recorder.record_diagnostic(
        log,
        DiagnosticInput {
            view,
            background: request.background,
            cost: Some(cost),
            calculation_failed: false,
            litellm_model_name: request.litellm_model_name.map(str::to_owned),
            router_model_id: router,
        },
    );
    HiddenOutcome::Found(cost)
}

// The kwargs step failed, so only the short failure diagnostic is recorded.
pub(super) fn preparation_failure(log: &mut LoggingState, message: &str) {
    let info = diagnostics::failure_information(message, None, &log.call_type);
    log.model_call_details
        .insert(diagnostics::FAILURE_KEY.to_owned(), info);
}

// The cost calculator path: billing identity, the injected calculation and its outcome.
async fn record_calculated<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    cache_hit: Option<bool>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let billing = match prepare(recorder, log, view, request, cache_hit) {
        Ok(found) => found,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            return None;
        }
    };
    let calculated = {
        let calculation = cost_request(&billing, log, view);
        recorder.calculator.calculate(&calculation).await
    };
    record_outcome(recorder, log, view, request, (billing, calculated))
}

fn record_outcome<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    outcome: (BillingIdentity, Result<Decimal, PortFailure>),
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let (billing, calculated) = outcome;
    let router_model_id = billing.router_model_id.clone();
    let calculation_failed = calculated.is_err();
    let total = match calculated {
        Ok(amount) => Some(identity::total_with_additional(
            amount,
            identity::additional_cost(log),
        )),
        Err(error) => {
            let info = diagnostics::failure_information(
                &error.to_string(),
                Some(&billing),
                &log.call_type,
            );
            log.model_call_details
                .insert(diagnostics::FAILURE_KEY.to_owned(), info);
            None
        }
    };
    recorder.record_diagnostic(
        log,
        DiagnosticInput {
            view,
            background: request.background,
            cost: total,
            calculation_failed,
            litellm_model_name: request.litellm_model_name.map(str::to_owned),
            router_model_id,
        },
    );
    total
}

// Every value response_cost_calculator reads, resolved before the calculator runs.
fn prepare<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &LoggingState,
    view: &ResponseView<'_>,
    request: &ViewRequest<'_>,
    cache_hit: Option<bool>,
) -> Result<BillingIdentity, PreparationError>
where
    Inference: ProviderInference,
{
    let router = match request.router_model_id.filter(|id| !id.is_empty()) {
        Some(router) => Some(router.to_owned()),
        None => match view::hidden_text(view, "model_id") {
            Some(hidden) => Some(hidden.to_owned()),
            None => identity::metadata::router_model_id(log)?,
        },
    };
    let model = request
        .litellm_model_name
        .filter(|name| !name.is_empty())
        .unwrap_or(&log.model)
        .to_owned();
    Ok(BillingIdentity {
        custom_llm_provider: identity::text_field(&log.model_call_details, "custom_llm_provider")?,
        base_model: identity::metadata::base_model(log)?,
        prompt: identity::prompt(log),
        data_residency: identity::text_field(&log.litellm_params, "data_residency")?,
        vertex_location: identity::vertex_location(log, &model, recorder.vertex, recorder.regions)?,
        region_name: identity::mantle_region(log, recorder.mantle)?,
        custom_pricing: recorder.custom_pricing(log, view)?,
        cache_hit,
        router_model_id: router,
        model,
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

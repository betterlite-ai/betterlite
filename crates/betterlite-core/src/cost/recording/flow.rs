use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::diagnostics::{self, DiagnosticInput};
use super::identity;
use super::view;
use super::{CostRecorder, PortFailure, RecordInput, RecordResponse};
use crate::cost::context::ProviderInference;
use crate::cost::prepare::ResponseView;

mod calculated;

// The per-call values every logging entry point passes, before anything is resolved.
pub(super) struct ViewRequest<'names> {
    pub(super) cache_hit: Option<bool>,
    pub(super) litellm_model_name: Option<&'names str>,
    pub(super) router_model_id: Option<&'names str>,
    pub(super) background: bool,
}

// What the response and the call log resolved before the calculator runs.
pub(super) struct Resolved<'names> {
    pub(super) cache_hit: Option<bool>,
    pub(super) custom_pricing: bool,
    pub(super) router_model_id: Option<&'names str>,
}

pub(super) struct Outcome {
    pub(super) billing: super::identity::BillingIdentity,
    pub(super) calculated: Result<Decimal, PortFailure>,
}

enum HiddenOutcome {
    Absent,
    Found(Decimal),
    Invalid,
}

impl<'names> ViewRequest<'names> {
    fn with_background(&self, background: bool) -> ViewRequest<'names> {
        ViewRequest {
            cache_hit: self.cache_hit,
            litellm_model_name: self.litellm_model_name,
            router_model_id: self.router_model_id,
            background,
        }
    }
}

// Ports _response_cost_calculator's dispatch over the response shapes LiteLLM accepts.
pub(super) async fn record<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    input: RecordInput<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let RecordInput {
        response,
        cache_hit,
        litellm_model_name,
        router_model_id,
    } = input;
    let request = ViewRequest {
        cache_hit,
        litellm_model_name,
        router_model_id,
        background: false,
    };
    match response {
        RecordResponse::Typed {
            response,
            background,
        } => {
            let view = ResponseView::Typed(response);
            record_view(recorder, log, &view, &request.with_background(background)).await
        }
        RecordResponse::Event(event) => record_event(recorder, log, event, &request).await,
        RecordResponse::Json(body) => {
            let scoped = request.with_background(view::background(body));
            record_body(recorder, log, body, &scoped).await
        }
        RecordResponse::GenerateContent(body) => {
            let scoped = request.with_background(view::background(body));
            record_generate_content(recorder, log, body, &scoped).await
        }
        RecordResponse::Absent => record_view(recorder, log, &ResponseView::Other, &request).await,
    }
}

async fn record_event<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    event: &Json,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    match view::event_view(event) {
        Ok(Some(parsed)) => record_view(recorder, log, &parsed, request).await,
        Ok(None) => record_view(recorder, log, &ResponseView::Other, request).await,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            None
        }
    }
}

async fn record_body<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    body: &Json,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    match view::body_view(body) {
        Ok(Some(parsed)) => record_view(recorder, log, &parsed, request).await,
        Ok(None) => record_view(recorder, log, &ResponseView::Other, request).await,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            None
        }
    }
}

// Ports _generate_content_result_as_model_response: a failed conversion leaves the body as it was.
async fn record_generate_content<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    body: &Json,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let conversion = recorder.conversion.convert(body, &log.model);
    match conversion {
        Ok(converted) => match view::body_view(&converted) {
            Ok(Some(parsed)) => record_view(recorder, log, &parsed, request).await,
            Ok(None) => record_body(recorder, log, body, request).await,
            Err(error) => {
                preparation_failure(log, &error.to_string());
                None
            }
        },
        Err(error) => {
            tracing::debug!(%error, "generate_content response cost normalization failed");
            record_body(recorder, log, body, request).await
        }
    }
}

async fn record_view<Inference>(
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
    calculated::record_calculated(recorder, log, view, request, cache_hit).await
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
    let value = view::hidden_params(view).and_then(|params| params.get("response_cost"));
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
    recorder.record_diagnostic(
        log,
        diagnostic_input(
            view,
            request,
            Some(cost),
            false,
            router_for_hidden(view, request),
        ),
    );
    HiddenOutcome::Found(cost)
}

fn diagnostic_input<'view>(
    view: &'view ResponseView<'view>,
    request: &ViewRequest<'_>,
    cost: Option<Decimal>,
    calculation_failed: bool,
    router_model_id: Option<String>,
) -> DiagnosticInput<'view> {
    DiagnosticInput {
        view,
        background: request.background,
        cost,
        calculation_failed,
        litellm_model_name: request.litellm_model_name.map(str::to_owned),
        router_model_id,
    }
}

fn router_for_hidden(view: &ResponseView<'_>, request: &ViewRequest<'_>) -> Option<String> {
    request
        .router_model_id
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| view::hidden_text(view, "model_id").map(str::to_owned))
}

// The kwargs step failed, so only the short failure diagnostic is recorded.
pub(super) fn preparation_failure(log: &mut LoggingState, message: &str) {
    let info = diagnostics::failure_information(message, None, &log.call_type);
    log.model_call_details
        .insert(diagnostics::FAILURE_KEY.to_owned(), info);
}

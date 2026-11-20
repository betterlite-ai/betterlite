use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::diagnostics::{self, DiagnosticInput};
use super::identity::{self, BillingIdentity, PreparationError};
use super::view::{self, JsonResponseError};
use super::{CostRecorder, CostRequest};
use crate::cost::context::ProviderInference;
use crate::cost::prepare::ResponseView;

// The per-call values every entry point passes, before anything is resolved.
pub(super) struct ViewRequest<'names> {
    pub(super) cache_hit: Option<bool>,
    pub(super) litellm_model_name: Option<&'names str>,
    pub(super) router_model_id: Option<&'names str>,
    pub(super) background: bool,
}

// One call's dependencies and inputs, so each step below names only what it adds.
pub(super) struct CostCall<'call, 'services, Inference> {
    pub(super) recorder: &'call CostRecorder<'services, Inference>,
    pub(super) log: &'call mut LoggingState,
    pub(super) view: &'call ResponseView<'call>,
    pub(super) request: &'call ViewRequest<'call>,
}

// A native generateContent body and its call, converted before any response view is built.
pub(super) struct GenerateContent<'call, 'services, Inference> {
    pub(super) recorder: &'call CostRecorder<'services, Inference>,
    pub(super) log: &'call mut LoggingState,
    pub(super) body: &'call Json,
    pub(super) request: &'call ViewRequest<'call>,
}

enum HiddenOutcome {
    Absent,
    Found(Decimal),
    Invalid,
}

impl<Inference> CostCall<'_, '_, Inference>
where
    Inference: ProviderInference,
{
    // Ports _response_cost_calculator's order: cache hit, unbilled call, hidden cost, then the calculator.
    pub(super) async fn run(mut self) -> Option<Decimal> {
        let cache_hit = self
            .request
            .cache_hit
            .or(identity::stored_cache_hit(self.log));
        if cache_hit == Some(true) || identity::unbilled(self.log, self.request.background) {
            return Some(Decimal::ZERO);
        }
        match self.hidden_cost() {
            HiddenOutcome::Absent => {}
            HiddenOutcome::Found(cost) => return Some(cost),
            HiddenOutcome::Invalid => return None,
        }
        self.calculated(cache_hit).await
    }

    // Ports the hidden-response-cost branch: it wins over the calculator and skips additional_response_cost.
    fn hidden_cost(&mut self) -> HiddenOutcome {
        let view = self.view;
        let value = view::typed(view)
            .and_then(|typed| typed.hidden_params)
            .and_then(|params| params.get("response_cost"));
        let cost = match value {
            None | Some(Json::Null) => return HiddenOutcome::Absent,
            Some(value) => match identity::amount(value) {
                Some(cost) => cost,
                None => {
                    preparation_failure(self.log, "the hidden response_cost must be a number");
                    return HiddenOutcome::Invalid;
                }
            },
        };
        let router = self
            .request
            .router_model_id
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .or_else(|| view::hidden_text(view, "model_id").map(str::to_owned));
        self.diagnose(Some(cost), false, router);
        HiddenOutcome::Found(cost)
    }

    // The cost calculator path: billing identity, the injected calculation and its outcome.
    async fn calculated(&mut self, cache_hit: Option<bool>) -> Option<Decimal> {
        let billing = match self.prepare(cache_hit) {
            Ok(found) => found,
            Err(error) => {
                preparation_failure(self.log, &error.to_string());
                return None;
            }
        };
        let calculated = {
            let calculation = cost_request(&billing, self.log, self.view);
            self.recorder.calculator.calculate(&calculation).await
        };
        let router_model_id = billing.router_model_id.clone();
        let calculation_failed = calculated.is_err();
        let total = match calculated {
            Ok(amount) => Some(identity::total_with_additional(
                amount,
                identity::additional_cost(self.log),
            )),
            Err(error) => {
                let info = identity::failure_information(
                    &error.to_string(),
                    Some(&billing),
                    &self.log.call_type,
                );
                self.log
                    .model_call_details
                    .insert(diagnostics::FAILURE_KEY.to_owned(), info);
                None
            }
        };
        self.diagnose(total, calculation_failed, router_model_id);
        total
    }

    fn diagnose(
        &mut self,
        cost: Option<Decimal>,
        calculation_failed: bool,
        router_model_id: Option<String>,
    ) {
        let input = DiagnosticInput {
            view: self.view,
            background: self.request.background,
            cost,
            calculation_failed,
            litellm_model_name: self.request.litellm_model_name.map(str::to_owned),
            router_model_id,
        };
        self.recorder.record_diagnostic(self.log, input);
    }

    // Every value response_cost_calculator reads, resolved before the calculator runs.
    fn prepare(&mut self, cache_hit: Option<bool>) -> Result<BillingIdentity, PreparationError> {
        let router = match self.request.router_model_id.filter(|id| !id.is_empty()) {
            Some(router) => Some(router.to_owned()),
            None => match view::hidden_text(self.view, "model_id") {
                Some(hidden) => Some(hidden.to_owned()),
                None => identity::metadata::router_model_id(self.log)?,
            },
        };
        let model = self
            .request
            .litellm_model_name
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.log.model)
            .to_owned();
        Ok(BillingIdentity {
            custom_llm_provider: identity::text_field(
                &self.log.model_call_details,
                "custom_llm_provider",
            )?,
            base_model: identity::metadata::base_model(self.log)?,
            prompt: identity::prompt(self.log),
            data_residency: identity::text_field(&self.log.litellm_params, "data_residency")?,
            vertex_location: identity::vertex_location(
                self.log,
                &model,
                self.recorder.vertex,
                self.recorder.regions,
            )?,
            region_name: identity::mantle_region(self.log, self.recorder.mantle)?,
            custom_pricing: self.recorder.custom_pricing(self.log, self.view)?,
            cache_hit,
            router_model_id: router,
            model,
        })
    }
}

// Runs one call over a parsed view; a view that failed to parse records only the short failure.
pub(super) async fn record_prepared<Inference>(
    recorder: &CostRecorder<'_, Inference>,
    log: &mut LoggingState,
    prepared: Result<Option<ResponseView<'_>>, JsonResponseError>,
    request: &ViewRequest<'_>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let parsed = match prepared {
        Ok(parsed) => parsed,
        Err(error) => {
            preparation_failure(log, &error.to_string());
            return None;
        }
    };
    let view = parsed.unwrap_or(ResponseView::Other);
    CostCall {
        recorder,
        log,
        view: &view,
        request,
    }
    .run()
    .await
}

// Ports _generate_content_result_as_model_response: a failed conversion leaves the body as it was.
pub(super) async fn record_generate_content<Inference>(
    call: GenerateContent<'_, '_, Inference>,
) -> Option<Decimal>
where
    Inference: ProviderInference,
{
    let GenerateContent {
        recorder,
        log,
        body,
        request,
    } = call;
    let converted = match recorder.conversion.convert(body, &log.model) {
        Ok(converted) => converted,
        Err(error) => {
            tracing::debug!(%error, "generate_content response cost normalization failed");
            return record_prepared(recorder, log, view::body_view(body), request).await;
        }
    };
    let prepared = match view::body_view(&converted) {
        Ok(Some(parsed)) => Ok(Some(parsed)),
        Ok(None) => view::body_view(body),
        Err(error) => Err(error),
    };
    record_prepared(recorder, log, prepared, request).await
}

// The kwargs step failed, so only the short failure diagnostic is recorded.
pub(super) fn preparation_failure(log: &mut LoggingState, message: &str) {
    let info = identity::failure_information(message, None, &log.call_type);
    log.model_call_details
        .insert(diagnostics::FAILURE_KEY.to_owned(), info);
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

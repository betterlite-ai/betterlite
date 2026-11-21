use std::collections::BTreeSet;
use std::sync::PoisonError;

use betterlite_error::proxy_rate_limit_error::{
    ProxyRateLimitProjectionError, RateLimitResetWindow,
};
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitDescriptor,
    RateLimitResponse, ReservationWindowIdentity,
};
use crate::provider_attribution::{RateLimitErrorAttribution, handle_rate_limit_error};
use crate::request_stash::{RequestStash, RequestStashSlot, StashState};
use crate::token_estimation::RequestData;
use crate::token_estimation::estimate::{
    InputOutputTokens, TokenEstimateRequest, estimate_precise_input_tokens, input_and_output_tokens,
};
use crate::token_estimation::output_cap::{
    CapEndpoint, apply_implicit_output_cap, explicit_output_cap,
};
use crate::token_estimation::services::{GenAiTranslation, InputTokenCounter, ResponsesInput};
use crate::token_reservations::{IoReservation, TokenReservationError, TokenReservations};
use crate::windowed_admission::OVER_LIMIT_CODE;

mod tests;

pub struct EstimationServices<'deps> {
    pub translation: &'deps dyn GenAiTranslation,
    pub responses: &'deps dyn ResponsesInput,
    pub counter: &'deps dyn InputTokenCounter,
}

pub struct ProjectAdmission<'deps> {
    pub reservations: TokenReservations<'deps>,
    pub gauges: &'deps ConcurrencyGauges,
    pub estimation: EstimationServices<'deps>,
}

// the combined-TPM reservation the caller made just before, rolled back when an IO bucket rejects the request
pub struct TpmReservation<'request> {
    pub scopes: &'request [(String, String)],
    pub amount: i64,
}

pub struct ProjectIoRequest<'request> {
    pub descriptors: &'request [RateLimitDescriptor],
    pub requested_model: Option<&'request str>,
    pub call_type: Option<&'request str>,
    pub endpoint: CapEndpoint,
    pub tpm_reservation: TpmReservation<'request>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectAdmissionError {
    #[error(transparent)]
    Reservation(#[from] TokenReservationError),
    #[error(transparent)]
    Projection(#[from] ProxyRateLimitProjectionError),
    // boxed like the token reservations: the 429 carries headers and a payload, far more than the failures beside it
    #[error("{0}")]
    RateLimited(Box<ProxyRateLimitError>),
}

impl ProjectAdmission<'_> {
    pub async fn reserve_project_io_tokens(
        &self,
        request: &ProjectIoRequest<'_>,
        data: &mut RequestData,
        stash_slot: &RequestStashSlot,
    ) -> Result<(), ProjectAdmissionError> {
        let stash = stash_slot.claim_for_call_id(request_call_id(data));
        let io_descriptors = project_io_descriptors(request.descriptors);
        if io_descriptors.is_empty() {
            return Ok(());
        }
        let smallest_otpm_limit = min_otpm_limit(&io_descriptors);
        let estimate = self
            .estimate_io_tokens(request, data, smallest_otpm_limit)
            .await;
        // the hard cap keeps an unbounded response inside the OTPM budget until post-call reconciliation runs
        apply_implicit_output_cap(
            data,
            smallest_otpm_limit,
            request.call_type,
            None,
            request.endpoint,
        );
        let reservation = self
            .reservations
            .reserve_io_tokens(
                &io_descriptors,
                estimate.input_tokens,
                estimate.output_tokens,
            )
            .await?;
        if reservation.response.overall_code == OVER_LIMIT_CODE {
            self.reject(request, &stash, &reservation.response).await?;
        }
        record_reservation(&stash, &io_descriptors, &reservation);
        tracing::debug!(
            itpm = reservation.itpm_reserved,
            otpm = reservation.otpm_reserved,
            model = request.requested_model,
            "ITPM/OTPM tokens reserved"
        );
        Ok(())
    }

    async fn estimate_io_tokens(
        &self,
        request: &ProjectIoRequest<'_>,
        data: &RequestData,
        smallest_otpm_limit: Option<i64>,
    ) -> InputOutputTokens {
        let services = &self.estimation;
        let output_request = TokenEstimateRequest {
            data,
            call_type: request.call_type,
            model: None,
            min_configured_tpm_limit: smallest_otpm_limit,
            configured_output_tokens: None,
        };
        let raw_output =
            input_and_output_tokens(services.translation, &output_request).output_tokens;
        let input_request = TokenEstimateRequest {
            data,
            call_type: request.call_type,
            model: request.requested_model,
            min_configured_tpm_limit: None,
            configured_output_tokens: None,
        };
        let raw_input = estimate_precise_input_tokens(
            services.translation,
            services.responses,
            services.counter,
            &input_request,
        )
        .await;
        // an explicit cap of 0 is a real zero-output request, so only an uncapped estimate gets the one-token floor
        let output_tokens = if explicit_output_cap(data, request.call_type).is_some() {
            raw_output
        } else {
            raw_output.max(1_i64)
        };
        InputOutputTokens {
            input_tokens: raw_input.max(1_i64),
            output_tokens,
        }
    }

    async fn reject(
        &self,
        request: &ProjectIoRequest<'_>,
        stash: &RequestStash,
        response: &RateLimitResponse,
    ) -> Result<(), ProjectAdmissionError> {
        let tpm = &request.tpm_reservation;
        if tpm.amount > 0 {
            self.reservations
                .refund_reserved_tokens(tpm.scopes, tpm.amount, &BTreeSet::new())
                .await?;
            // the failure hook sees the same stash after the 429 and must not refund this reservation again
            stash
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reservation_released = true;
        }
        self.gauges
            .release_stashed_parallel_slot(Some(stash), false)
            .await;
        let settlement = &self.reservations.settlement;
        let window = RateLimitResetWindow {
            now: settlement.clock.current_time(),
            window_size: settlement.default_window_size,
        };
        let attribution = RateLimitErrorAttribution {
            requested_model: request.requested_model,
            native: self.reservations.attribution.native,
            router: self.reservations.attribution.router,
        };
        // LiteLLM's handler raises only for an OVER_LIMIT status, so a response without one falls through to the stash
        let Some(error) =
            handle_rate_limit_error(response, request.descriptors, window, &attribution)?
        else {
            return Ok(());
        };
        Err(ProjectAdmissionError::RateLimited(Box::new(error)))
    }
}

fn request_call_id(data: &RequestData) -> Option<&str> {
    data.get("litellm_call_id").and_then(|value| match value {
        Json::Text(id) => Some(id.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    })
}

fn project_io_descriptors(descriptors: &[RateLimitDescriptor]) -> Vec<RateLimitDescriptor> {
    descriptors
        .iter()
        .filter(|descriptor| {
            matches!(
                descriptor.key.as_str(),
                PROJECT_ITPM_DESCRIPTOR_KEY | PROJECT_OTPM_DESCRIPTOR_KEY
            )
        })
        .cloned()
        .collect()
}

// the smallest OTPM limit bounds the output floor, the way the combined-TPM path bounds it by the smallest TPM limit
fn min_otpm_limit(io_descriptors: &[RateLimitDescriptor]) -> Option<i64> {
    io_descriptors
        .iter()
        .filter(|descriptor| descriptor.key == PROJECT_OTPM_DESCRIPTOR_KEY)
        .filter_map(|descriptor| descriptor.rate_limit.as_ref()?.tokens_per_unit)
        .min()
}

fn record_reservation(
    stash: &RequestStash,
    io_descriptors: &[RateLimitDescriptor],
    reservation: &IoReservation,
) {
    let windows = reservation.response.reservation_windows.as_ref();
    let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    if reservation.itpm_reserved > 0 {
        state.itpm_reserved_tokens = reservation.itpm_reserved;
        state.itpm_reserved_scopes = bucket_scopes(io_descriptors, PROJECT_ITPM_DESCRIPTOR_KEY);
        state.itpm_reserved_windows = bucket_windows(windows, PROJECT_ITPM_DESCRIPTOR_KEY);
    }
    if reservation.otpm_reserved > 0 {
        state.otpm_reserved_tokens = reservation.otpm_reserved;
        state.otpm_reserved_scopes = bucket_scopes(io_descriptors, PROJECT_OTPM_DESCRIPTOR_KEY);
        state.otpm_reserved_windows = bucket_windows(windows, PROJECT_OTPM_DESCRIPTOR_KEY);
    }
    merge_statuses(&mut state, &reservation.response);
    drop(state);
}

// the stored windowed response gains the IO statuses so the quota headers carry both limits
fn merge_statuses(state: &mut StashState, response: &RateLimitResponse) {
    if let Some(stored) = state.rate_limit_response.as_mut() {
        stored.statuses.extend(response.statuses.iter().cloned());
        return;
    }
    if !response.statuses.is_empty() {
        state.rate_limit_response = Some(response.clone());
    }
}

fn bucket_scopes(descriptors: &[RateLimitDescriptor], bucket: &str) -> BTreeSet<(String, String)> {
    descriptors
        .iter()
        .filter(|descriptor| descriptor.key == bucket)
        .map(|descriptor| (descriptor.key.clone(), descriptor.value.clone()))
        .collect()
}

// LiteLLM keeps the windows whose counter key names the bucket, a substring match on the key
fn bucket_windows(
    windows: Option<&BTreeSet<ReservationWindowIdentity>>,
    bucket: &str,
) -> BTreeSet<ReservationWindowIdentity> {
    windows
        .into_iter()
        .flatten()
        .filter(|window| window.counter_key.contains(bucket))
        .cloned()
        .collect()
}

use std::collections::BTreeSet;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::proxy_rate_limit_error::{
    ProxyRateLimitProjectionError, RateLimitResetWindow,
};
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::atomic_counter::payload::IncrementAmounts;
use crate::limiter_records::{RateLimitDescriptor, RateLimitResponse, ReservationWindowIdentity};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::provider_attribution::{RateLimitErrorAttribution, handle_rate_limit_error};
use crate::settlement_and_rollback::{AtomicSettlement, SettlementError};
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

mod io_filtering;
mod refund_project;
mod refund_tpm;
mod tpm_filtering;

mod both_succeeded;
mod frame_descriptor;
mod frame_early;
mod frame_no_descriptors;
mod frame_over_limit;
mod frame_reserve;
mod itpm_only;
mod itpm_over_limit;
mod itpm_reserved;
mod otpm_over_limit;
mod otpm_rollback;
mod refund_early;
mod signatures;
mod tests;
mod tpm_atomic;

pub struct TokenReservations<'deps> {
    pub settlement: AtomicSettlement<'deps>,
    pub adjustments: &'deps TokenAdjustments,
    pub descriptors: &'deps ModelDescriptorBuilder,
    pub attribution: RateLimitErrorAttribution<'deps>,
}

pub struct IoReservation {
    pub response: RateLimitResponse,
    pub itpm_reserved: i64,
    pub otpm_reserved: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenReservationError {
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    #[error(transparent)]
    Tokens(#[from] TokenAdjustmentError),
    #[error(transparent)]
    Projection(#[from] ProxyRateLimitProjectionError),
    // boxed like session budgets: the 429 carries headers and a payload, far more than the failures beside it
    #[error("{0}")]
    RateLimited(Box<ProxyRateLimitError>),
}

fn empty_response() -> RateLimitResponse {
    RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses: Vec::new(),
        reservation_windows: None,
    }
}

fn merged_io_response(
    itpm_response: &RateLimitResponse,
    otpm_response: &RateLimitResponse,
) -> RateLimitResponse {
    let mut statuses = itpm_response.statuses.clone();
    statuses.extend(otpm_response.statuses.iter().cloned());
    let mut windows = itpm_response
        .reservation_windows
        .clone()
        .unwrap_or_default();
    if let Some(otpm_windows) = &otpm_response.reservation_windows {
        windows.extend(otpm_windows.iter().cloned());
    }
    RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: Some(windows),
    }
}

impl TokenReservations<'_> {
    pub async fn reserve_tpm_tokens(
        &self,
        descriptors: &[RateLimitDescriptor],
        estimated_tokens: i64,
    ) -> Result<RateLimitResponse, TokenReservationError> {
        let tpm_descriptors = tpm_filtering::tpm_descriptors(descriptors);
        if tpm_descriptors.is_empty() {
            return Ok(empty_response());
        }
        self.charge_token_bucket(&tpm_descriptors, estimated_tokens)
            .await
    }

    pub async fn reserve_io_tokens(
        &self,
        descriptors: &[RateLimitDescriptor],
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<IoReservation, TokenReservationError> {
        let (itpm_descriptors, otpm_descriptors) = io_filtering::split_io_descriptors(descriptors);
        if itpm_descriptors.is_empty() && otpm_descriptors.is_empty() {
            return Ok(IoReservation {
                response: empty_response(),
                itpm_reserved: 0,
                otpm_reserved: 0,
            });
        }
        let itpm_response = self
            .charge_token_bucket(&itpm_descriptors, estimated_input_tokens)
            .await?;
        if itpm_response.overall_code == "OVER_LIMIT" {
            return Ok(IoReservation {
                response: itpm_response,
                itpm_reserved: 0,
                otpm_reserved: 0,
            });
        }
        let itpm_reserved = if itpm_descriptors.is_empty() {
            0
        } else {
            estimated_input_tokens
        };
        if otpm_descriptors.is_empty() {
            return Ok(IoReservation {
                response: itpm_response,
                itpm_reserved,
                otpm_reserved: 0,
            });
        }
        self.reserve_output_bucket(
            &itpm_descriptors,
            itpm_reserved,
            &itpm_response,
            &otpm_descriptors,
            estimated_output_tokens,
        )
        .await
    }

    async fn reserve_output_bucket(
        &self,
        itpm_descriptors: &[RateLimitDescriptor],
        itpm_reserved: i64,
        itpm_response: &RateLimitResponse,
        otpm_descriptors: &[RateLimitDescriptor],
        estimated_output_tokens: i64,
    ) -> Result<IoReservation, TokenReservationError> {
        let otpm_response = self
            .charge_token_bucket(otpm_descriptors, estimated_output_tokens)
            .await?;
        if otpm_response.overall_code == "OVER_LIMIT" {
            self.refund_itpm_after_otpm_failure(itpm_descriptors, itpm_reserved, itpm_response)
                .await?;
            return Ok(IoReservation {
                response: otpm_response,
                itpm_reserved: 0,
                otpm_reserved: 0,
            });
        }
        Ok(IoReservation {
            response: merged_io_response(itpm_response, &otpm_response),
            itpm_reserved,
            otpm_reserved: estimated_output_tokens,
        })
    }

    pub async fn refund_reserved_tokens(
        &self,
        scopes: &[(String, String)],
        amount: i64,
        reservation_windows: &BTreeSet<ReservationWindowIdentity>,
    ) -> Result<(), TokenReservationError> {
        if amount <= 0 || scopes.is_empty() {
            return Ok(());
        }
        let window_size = self.settlement.default_window_size;
        if reservation_windows.is_empty() {
            return refund_tpm::refund_through_tpm_ops(
                self.adjustments,
                scopes,
                amount,
                window_size,
            )
            .await
            .map_err(TokenReservationError::from);
        }
        refund_project::refund_through_project_ops(
            self.adjustments,
            scopes,
            amount,
            reservation_windows,
            window_size,
        )
        .await
        .map_err(TokenReservationError::from)
    }

    pub async fn enforce_frame_project_io_quota(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        requested_model: Option<&str>,
        estimated_input_tokens: i64,
        estimated_output_tokens: i64,
    ) -> Result<(), TokenReservationError> {
        let Some(auth) = auth else {
            return Ok(());
        };
        let mut descriptors = Vec::new();
        self.descriptors
            .project_io_descriptors(auth, requested_model, &mut descriptors);
        if descriptors.is_empty() {
            return Ok(());
        }
        let reservation = self
            .reserve_io_tokens(
                &descriptors,
                estimated_input_tokens,
                estimated_output_tokens,
            )
            .await?;
        let Some(error) =
            self.frame_rejection(requested_model, &descriptors, &reservation.response)?
        else {
            return Ok(());
        };
        Err(TokenReservationError::RateLimited(Box::new(error)))
    }

    async fn charge_token_bucket(
        &self,
        descriptors: &[RateLimitDescriptor],
        tokens: i64,
    ) -> Result<RateLimitResponse, TokenReservationError> {
        let increments: Vec<IncrementAmounts> = descriptors
            .iter()
            .map(|_descriptor| IncrementAmounts {
                requests: 0,
                tokens,
            })
            .collect();
        self.settlement
            .atomic_check_and_increment_by_n(descriptors, &increments)
            .await
            .map_err(TokenReservationError::from)
    }

    async fn refund_itpm_after_otpm_failure(
        &self,
        itpm_descriptors: &[RateLimitDescriptor],
        itpm_reserved: i64,
        itpm_response: &RateLimitResponse,
    ) -> Result<(), TokenReservationError> {
        let scopes: Vec<(String, String)> = itpm_descriptors
            .iter()
            .map(|descriptor| (descriptor.key.clone(), descriptor.value.clone()))
            .collect();
        let windows = itpm_response
            .reservation_windows
            .clone()
            .unwrap_or_default();
        self.refund_reserved_tokens(&scopes, itpm_reserved, &windows)
            .await
    }

    fn frame_rejection(
        &self,
        requested_model: Option<&str>,
        descriptors: &[RateLimitDescriptor],
        response: &RateLimitResponse,
    ) -> Result<Option<ProxyRateLimitError>, TokenReservationError> {
        if response.overall_code != "OVER_LIMIT" {
            return Ok(None);
        }
        let attribution = RateLimitErrorAttribution {
            requested_model,
            native: self.attribution.native,
            router: self.attribution.router,
        };
        let window = RateLimitResetWindow {
            now: self.settlement.clock.current_time(),
            window_size: self.settlement.default_window_size,
        };
        handle_rate_limit_error(response, descriptors, window, &attribution)
            .map_err(TokenReservationError::from)
    }
}

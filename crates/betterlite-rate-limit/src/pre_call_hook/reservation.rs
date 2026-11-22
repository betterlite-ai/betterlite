use betterlite_auth::auth_limit_inputs::estimates::get_estimated_output_tokens;

use crate::limiter_records::RateLimitResponse;
use crate::pre_call_hook::PreCallHookError;
use crate::pre_call_hook::common::{
    configured_tpm_limits, project_rate_limit, reserved_scopes, store_tpm_reservation,
    token_reservations,
};
use crate::pre_call_hook::flow::FlowContext;
use crate::project_admission::{
    EstimationServices, ProjectAdmission, ProjectIoRequest, TpmReservation,
};
use crate::token_estimation::estimate::{TokenEstimateRequest, estimate_tokens_for_request};
use crate::token_estimation::output_cap::apply_implicit_output_cap;
use crate::windowed_admission::OVER_LIMIT_CODE;

#[derive(Debug, Default)]
pub(super) struct TpmReservationState {
    pub scopes: Vec<(String, String)>,
    pub amount: i64,
}

pub(super) async fn reserve_tokens(
    context: &mut FlowContext<'_>,
) -> Result<TpmReservationState, PreCallHookError> {
    if !context.hook.assembly.settings.tpm_reservation_enabled {
        return Ok(TpmReservationState::default());
    }
    let Some(smallest) = configured_tpm_limits(context.descriptors).into_iter().min() else {
        return Ok(TpmReservationState::default());
    };
    let configured_output_tokens =
        get_estimated_output_tokens(context.auth, context.requested_model);
    apply_implicit_output_cap(
        &mut *context.data,
        Some(smallest),
        context.call_type_name,
        configured_output_tokens,
        context.endpoint,
    );
    let estimate = estimate_tokens(context, smallest, configured_output_tokens);
    let reservations = token_reservations(context.hook, context.model, context.requested_model);
    let response = reservations
        .reserve_tpm_tokens(context.descriptors, estimate)
        .await?;
    if response.overall_code == OVER_LIMIT_CODE {
        return reject(context, &response).await;
    }
    Ok(finish_reservation(context, estimate, &response))
}

fn finish_reservation(
    context: &FlowContext<'_>,
    estimate: i64,
    response: &RateLimitResponse,
) -> TpmReservationState {
    let scopes = reserved_scopes(context.descriptors);
    let model = context.model.rated_model(context.requested_model);
    store_tpm_reservation(context.stash, estimate, model, &scopes, response);
    tracing::debug!(
        tokens = estimate,
        model = context.requested_model,
        "TPM tokens reserved"
    );
    TpmReservationState {
        scopes,
        amount: estimate,
    }
}

pub(super) async fn reserve_project_io(
    context: &mut FlowContext<'_>,
    tpm: &TpmReservationState,
) -> Result<(), PreCallHookError> {
    let admission = ProjectAdmission {
        reservations: token_reservations(context.hook, context.model, context.requested_model),
        gauges: context.hook.assembly.gauges.as_ref(),
        estimation: EstimationServices {
            translation: context.hook.translation.as_ref(),
            responses: context.hook.responses.as_ref(),
            counter: context.hook.input_tokens.as_ref(),
        },
    };
    let request = ProjectIoRequest {
        descriptors: context.descriptors,
        requested_model: context.requested_model,
        call_type: context.call_type_name,
        endpoint: context.endpoint,
        tpm_reservation: TpmReservation {
            scopes: &tpm.scopes,
            amount: tpm.amount,
        },
    };
    admission
        .reserve_project_io_tokens(&request, &mut *context.data, context.stash_slot)
        .await?;
    Ok(())
}

async fn reject(
    context: &mut FlowContext<'_>,
    response: &RateLimitResponse,
) -> Result<TpmReservationState, PreCallHookError> {
    context
        .hook
        .assembly
        .gauges
        .release_stashed_parallel_slot(Some(context.stash), false)
        .await;
    let Some(error) = project_rate_limit(
        context.hook,
        response,
        context.descriptors,
        context.requested_model,
    )?
    else {
        return Ok(TpmReservationState::default());
    };
    Err(PreCallHookError::RateLimited(Box::new(error)))
}

fn estimate_tokens(
    context: &FlowContext<'_>,
    smallest: i64,
    configured_output_tokens: Option<i64>,
) -> i64 {
    let request = TokenEstimateRequest {
        data: &*context.data,
        call_type: context.call_type_name,
        model: context.requested_model,
        min_configured_tpm_limit: Some(smallest),
        configured_output_tokens,
    };
    estimate_tokens_for_request(context.hook.translation.as_ref(), &request).max(1_i64)
}

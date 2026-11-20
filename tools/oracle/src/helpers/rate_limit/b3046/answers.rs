use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_core::api::batch_enqueued_reserve::response::BatchResponseView;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_rate_limit::post_call_settlement::failure_event::FailureEvent;
use betterlite_rate_limit::post_call_settlement::failure_hook::FailureHook;
use betterlite_rate_limit::post_call_settlement::logging_hook::{LoggingTargets, logging_hook};
use betterlite_rate_limit::post_call_settlement::success_event::SuccessCallback;
use betterlite_rate_limit::post_call_settlement::success_hook::SuccessHook;
use betterlite_rate_limit::reconciliation_operations::io_token::{
    IoTokenReconciliation, IoTokenResponse,
};
use betterlite_rate_limit::reconciliation_operations::success_event::{
    SuccessEvent, SuccessResponse,
};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::cases::Case;
use super::fakes::WINDOW;
use super::fixtures::Fixture;

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()
}

pub(crate) fn logging_answer(case: &Case, mirror: bool) -> Value {
    let fixture = Fixture::new(case);
    let mut slp_headers = case.headers.clone();
    let mut response_headers = AdditionalHeaders::new();
    let runs = if mirror { case.runs.unwrap_or(1) } else { 1 };
    for _ in 0..runs {
        let targets = LoggingTargets {
            standard_logging_object: Some(&mut slp_headers),
            response_hidden_params: mirror.then_some(&mut response_headers),
        };
        logging_hook(&fixture.slot, case.call_id.as_deref(), targets);
    }
    let mirror_headers = mirror.then_some(response_headers);
    fixture.record(Some(&slp_headers), mirror_headers.as_ref())
}

pub(crate) fn success_event_answer(case: &Case) -> HelperResult {
    let fixture = Fixture::new(case);
    let runtime = runtime()?;
    let callback = success_callback(case, &fixture);
    runtime.block_on(fixture.settlement().log_success_event(&callback));
    Ok(fixture.record(None, None))
}

fn success_callback<'case>(case: &'case Case, fixture: &'case Fixture) -> SuccessCallback<'case> {
    let event = SuccessEvent {
        response: SuccessResponse::Unmodelled,
        combined_usage: case.usage.as_ref(),
        rate_limit_type: TokenRateLimitType::Total,
        logging: &case.logging,
        request_metadata: &fixture.metadata,
        request_user: None,
        model_group: None,
        stash: &fixture.slot,
        call_id: case.call_id.as_deref(),
        models: &fixture.models,
        window_size: WINDOW,
    };
    let io = IoTokenReconciliation {
        response: IoTokenResponse::Rerank {
            tokens: None,
            billed_total: None,
        },
        combined_usage: case.usage.as_ref(),
        window_size: WINDOW,
    };
    SuccessCallback { event, io }
}

pub(crate) fn failure_event_answer(case: &Case) -> HelperResult {
    let fixture = Fixture::new(case);
    let runtime = runtime()?;
    let event = FailureEvent {
        stash: &fixture.slot,
        call_id: case.call_id.as_deref(),
        combined_usage: case.usage.as_ref(),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    };
    runtime.block_on(fixture.settlement().log_failure_event(&event));
    Ok(fixture.record(None, None))
}

pub(crate) fn disconnect_answer(case: &Case) -> HelperResult {
    let fixture = Fixture::new(case);
    let runtime = runtime()?;
    let settle = fixture.settlement();
    runtime.block_on(settle.release_slot_on_disconnect(&fixture.slot));
    Ok(fixture.record(None, None))
}

fn batch_view(case: &Case) -> Option<BatchResponseView> {
    let response = case.response.as_ref()?;
    if response.object.as_deref() != Some("batch") {
        return None;
    }
    let status = response.status?;
    Some(BatchResponseView {
        id: response.id.clone(),
        status,
    })
}

fn hidden_headers(case: &Case) -> Option<AdditionalHeaders> {
    let response = case.response.as_ref()?;
    let hidden = response.hidden_params.as_ref()?;
    Some(hidden.additional_headers.clone())
}

pub(crate) fn success_hook_answer(case: &Case) -> HelperResult {
    let fixture = Fixture::new(case);
    let runtime = runtime()?;
    let view = batch_view(case);
    let mut headers = hidden_headers(case);
    let hook = SuccessHook {
        stash: &fixture.slot,
        call_id: case.call_id.as_deref(),
        response: view.as_ref(),
    };
    let settle = fixture.settlement();
    runtime.block_on(settle.post_call_success_hook(&hook, headers.as_mut()));
    Ok(fixture.record(None, headers.as_ref()))
}

const fn failure_hook<'case>(case: &'case Case, fixture: &'case Fixture) -> FailureHook<'case> {
    FailureHook {
        stash: &fixture.slot,
        combined_usage: case.usage.as_ref(),
        rate_limit_type: TokenRateLimitType::Total,
        window_size: WINDOW,
    }
}

pub(crate) fn failure_hook_answer(case: &Case) -> HelperResult {
    let fixture = Fixture::new(case);
    let runtime = runtime()?;
    let hook = failure_hook(case, &fixture);
    runtime.block_on(fixture.settlement().post_call_failure_hook(&hook));
    Ok(fixture.record(None, None))
}

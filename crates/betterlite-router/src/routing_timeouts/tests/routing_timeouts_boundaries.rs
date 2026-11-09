use std::time::Instant;

use super::{at, global_timeout, no_deployment, request_settings, router_settings, seconds};
use crate::routing_timeouts::completion::{TimeoutKwargs, TimeoutPhases, TimeoutValue, resolve};
use crate::routing_timeouts::error::TimeoutError;
use crate::routing_timeouts::plan::{RouterTimeoutSources, TimeoutPlan, resolve_router_timeouts};
use crate::routing_timeouts::seconds::Seconds;

#[test]
fn negative_seconds_is_an_error() {
    let error = Seconds::new("model_list.0.litellm_params.timeout", -1.0).unwrap_err();
    assert_eq!(
        error,
        TimeoutError::Negative {
            field: "model_list.0.litellm_params.timeout".to_owned(),
            seconds: -1.0_f64,
        }
    );
    assert!(
        error
            .to_string()
            .contains("model_list.0.litellm_params.timeout")
    );
}

#[test]
fn nan_seconds_is_an_error() {
    let error = Seconds::new("router_settings.timeout", f64::NAN).unwrap_err();
    assert!(matches!(error, TimeoutError::NotFinite { .. }));
    assert!(error.to_string().contains("router_settings.timeout"));
}

#[test]
fn infinite_seconds_is_an_error() {
    let error = Seconds::new("stream_timeout", f64::INFINITY).unwrap_err();
    assert!(matches!(error, TimeoutError::NotFinite { .. }));
}

#[test]
fn huge_seconds_is_an_error() {
    let error = seconds(1e300)
        .duration("router_settings.timeout")
        .unwrap_err();
    assert!(matches!(error, TimeoutError::OutOfRange { .. }));
    assert!(error.to_string().contains("router_settings.timeout"));
}

#[test]
fn zero_seconds_is_kept() {
    let value = resolve(None, &TimeoutKwargs::default(), Some(seconds(0.0)), true);
    assert_eq!(value, TimeoutValue::Seconds(seconds(0.0)));
    assert_eq!(value.transport(), seconds(0.0));
}

#[test]
fn phases_transport_uses_read_or_600() {
    let with_read = TimeoutValue::Phases(TimeoutPhases {
        connect: Some(seconds(1.0)),
        read: Some(seconds(2.5)),
        write: Some(seconds(3.0)),
        pool: Some(seconds(4.0)),
    });
    assert_eq!(with_read.transport(), seconds(2.5));
    let without_read = TimeoutValue::Phases(TimeoutPhases {
        connect: Some(seconds(1.0)),
        read: None,
        write: Some(seconds(3.0)),
        pool: Some(seconds(4.0)),
    });
    assert_eq!(without_read.transport(), seconds(600.0));
}

#[test]
fn zero_attempt_expires_immediately() {
    let base = Instant::now();
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(0.0));
    let plan = TimeoutPlan::new(
        base,
        &router_settings(10.0),
        &request_settings(false),
        &deployment,
    )
    .unwrap();
    assert_eq!(plan.attempt_deadline(base).unwrap(), base);
    assert_eq!(plan.expired(base), TimeoutError::AttemptExpired);
}

#[test]
fn attempt_equal_to_deadline() {
    let base = Instant::now();
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(
        base,
        &router_settings(5.0),
        &request_settings(false),
        &deployment,
    )
    .unwrap();
    assert_eq!(plan.attempt_deadline(base).unwrap(), plan.deadline);
    assert_eq!(plan.expired(at(base, 5)), TimeoutError::DeadlineExpired);
}

#[test]
fn later_attempt_is_clamped_to_the_deadline() {
    let base = Instant::now();
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(4.0));
    let plan = TimeoutPlan::new(
        base,
        &router_settings(10.0),
        &request_settings(false),
        &deployment,
    )
    .unwrap();
    assert_eq!(plan.deadline, at(base, 10));
    assert_eq!(plan.attempt, seconds(4.0));
    assert_eq!(plan.attempt_deadline(at(base, 8)).unwrap(), at(base, 10));
}

#[test]
fn deadline_reached_at_now() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    assert_eq!(
        plan.attempt_deadline(at(base, 5)),
        Err(TimeoutError::DeadlineExpired)
    );
    assert_eq!(
        plan.remaining(at(base, 5)),
        Err(TimeoutError::DeadlineExpired)
    );
}

#[test]
fn cancelled_blocks_attempt() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    plan.cancellation.cancel();
    assert_eq!(plan.attempt_deadline(base), Err(TimeoutError::Cancelled));
}

#[test]
fn cancelled_blocks_remaining() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    plan.cancellation.cancel();
    assert_eq!(plan.remaining(base), Err(TimeoutError::Cancelled));
}

#[test]
fn cancelled_expired_is_cancelled() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    plan.cancellation.cancel();
    assert_eq!(plan.expired(at(base, 9)), TimeoutError::Cancelled);
}

#[test]
fn expired_before_the_deadline_is_attempt() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    let error = plan.expired(at(base, 4));
    assert_eq!(error, TimeoutError::AttemptExpired);
    assert_eq!(error.status_code(), Some(408));
}

#[test]
fn expired_at_the_deadline_is_deadline() {
    let base = Instant::now();
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    let plan = TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
    let error = plan.expired(at(base, 5));
    assert_eq!(error, TimeoutError::DeadlineExpired);
    assert_eq!(error.status_code(), Some(408));
}

#[test]
fn global_default_flags() {
    assert_eq!(global_timeout(6000.0, false).configured(), None);
    assert_eq!(
        global_timeout(6000.0, true).configured(),
        Some(seconds(6000.0))
    );
    assert_eq!(
        global_timeout(300.0, false).configured(),
        Some(seconds(300.0))
    );
    assert_eq!(global_timeout(0.0, true).configured(), Some(seconds(0.0)));
}

#[test]
fn router_constructor_fields() {
    let sources = RouterTimeoutSources {
        timeout: Some(seconds(7.0)),
        stream_timeout: Some(seconds(3.0)),
        default_stream_timeout: Some(seconds(4.0)),
    };
    let resolved = resolve_router_timeouts(&sources, global_timeout(6000.0, true));
    assert_eq!(resolved.timeout, seconds(7.0));
    assert_eq!(resolved.stream_timeout, Some(seconds(3.0)));
    assert_eq!(resolved.request_timeout, Some(seconds(6000.0)));
    assert_eq!(resolved.default_stream_timeout, Some(seconds(4.0)));
}

#[test]
fn router_without_timeout_uses_global() {
    let resolved = resolve_router_timeouts(
        &RouterTimeoutSources::default(),
        global_timeout(300.0, true),
    );
    assert_eq!(resolved.timeout, seconds(300.0));
    assert_eq!(resolved.request_timeout, None);
}

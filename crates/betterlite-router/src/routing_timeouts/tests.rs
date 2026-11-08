#![cfg(test)]

use std::time::{Duration, Instant};

use rstest::rstest;

use super::completion::{
    TimeoutKwargs, TimeoutPhases, TimeoutValue, resolve, supports_httpx_timeout,
};
use super::error::TimeoutError;
use super::global::GlobalRequestTimeout;
use super::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, RouterTimeouts, TimeoutPlan,
    attempt_timeout, non_stream_timeout, resolve_router_timeouts, stream_timeout,
};
use super::seconds::Seconds;

fn seconds(value: f64) -> Seconds {
    Seconds::new("test", value).unwrap()
}

fn global_timeout(value: f64, explicitly_set: bool) -> GlobalRequestTimeout {
    GlobalRequestTimeout {
        seconds: seconds(value),
        explicitly_set,
    }
}

fn router_settings(timeout: f64) -> RouterTimeouts {
    RouterTimeouts {
        timeout: seconds(timeout),
        stream_timeout: None,
        request_timeout: None,
        default_timeout: None,
        default_stream_timeout: None,
    }
}

fn request_settings(stream: bool) -> RequestTimeouts {
    RequestTimeouts {
        stream,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    }
}

fn no_deployment() -> DeploymentTimeouts {
    DeploymentTimeouts::default()
}

fn at(base: Instant, seconds: u64) -> Instant {
    base.checked_add(Duration::from_secs(seconds)).unwrap()
}

#[rstest]
#[case::completion_model_beats_kwargs("completion_model_beats_kwargs")]
#[case::completion_kwargs_timeout_beats_request_timeout(
    "completion_kwargs_timeout_beats_request_timeout"
)]
#[case::completion_request_timeout("completion_request_timeout")]
#[case::completion_configured_global("completion_configured_global")]
#[case::completion_unset_falls_back_to_600("completion_unset_falls_back_to_600")]
#[case::completion_explicit_6000_is_honoured("completion_explicit_6000_is_honoured")]
#[case::completion_runtime_300_is_explicit("completion_runtime_300_is_explicit")]
#[case::completion_phases_kept_for_httpx_provider("completion_phases_kept_for_httpx_provider")]
#[case::completion_phases_collapse_to_read("completion_phases_collapse_to_read")]
#[case::completion_phases_without_read_fall_back_to_600(
    "completion_phases_without_read_fall_back_to_600"
)]
#[case::stream_kwargs_beat_every_other_source("stream_kwargs_beat_every_other_source")]
#[case::stream_deployment_beats_router("stream_deployment_beats_router")]
#[case::stream_router_stream_timeout("stream_router_stream_timeout")]
#[case::stream_falls_back_to_non_stream("stream_falls_back_to_non_stream")]
#[case::non_stream_kwargs_timeout_wins("non_stream_kwargs_timeout_wins")]
#[case::non_stream_kwargs_request_timeout_wins("non_stream_kwargs_request_timeout_wins")]
#[case::non_stream_deployment_timeout("non_stream_deployment_timeout")]
#[case::non_stream_deployment_request_timeout("non_stream_deployment_request_timeout")]
#[case::non_stream_router_request_timeout("non_stream_router_request_timeout")]
#[case::non_stream_router_timeout("non_stream_router_timeout")]
#[case::non_stream_global_default("non_stream_global_default")]
#[case::plan_deadline_keeps_request_sources("plan_deadline_keeps_request_sources")]
#[case::plan_stream_deadline_and_attempt("plan_stream_deadline_and_attempt")]
fn routing_timeouts_behaviors(#[case] name: &str) {
    let mut kwargs = TimeoutKwargs::default();
    match name {
        "completion_model_beats_kwargs" => {
            kwargs.timeout = Some(seconds(22.0));
            kwargs.request_timeout = Some(seconds(33.0));
            let value = resolve(
                Some(TimeoutValue::Seconds(seconds(11.0))),
                &kwargs,
                Some(seconds(44.0)),
                true,
            );
            assert_eq!(value, TimeoutValue::Seconds(seconds(11.0)));
        }
        "completion_kwargs_timeout_beats_request_timeout" => {
            kwargs.timeout = Some(seconds(22.0));
            kwargs.request_timeout = Some(seconds(33.0));
            let value = resolve(None, &kwargs, Some(seconds(44.0)), true);
            assert_eq!(value, TimeoutValue::Seconds(seconds(22.0)));
        }
        "completion_request_timeout" => {
            kwargs.request_timeout = Some(seconds(33.0));
            let value = resolve(None, &kwargs, Some(seconds(44.0)), true);
            assert_eq!(value, TimeoutValue::Seconds(seconds(33.0)));
        }
        "completion_configured_global" => {
            let value = resolve(None, &kwargs, Some(seconds(44.0)), true);
            assert_eq!(value, TimeoutValue::Seconds(seconds(44.0)));
        }
        "completion_unset_falls_back_to_600" => {
            let value = resolve(
                None,
                &kwargs,
                global_timeout(6000.0, false).configured(),
                true,
            );
            assert_eq!(value, TimeoutValue::Seconds(seconds(600.0)));
        }
        "completion_explicit_6000_is_honoured" => {
            let value = resolve(
                None,
                &kwargs,
                global_timeout(6000.0, true).configured(),
                true,
            );
            assert_eq!(value, TimeoutValue::Seconds(seconds(6000.0)));
        }
        "completion_runtime_300_is_explicit" => {
            let value = resolve(
                None,
                &kwargs,
                global_timeout(300.0, false).configured(),
                true,
            );
            assert_eq!(value, TimeoutValue::Seconds(seconds(300.0)));
        }
        "completion_phases_kept_for_httpx_provider" => {
            let declared = TimeoutValue::Phases(phases(1.0, 2.0, 3.0, 4.0));
            assert!(supports_httpx_timeout("openai"));
            let value = resolve(
                Some(declared),
                &kwargs,
                None,
                supports_httpx_timeout("openai"),
            );
            assert_eq!(value, declared);
        }
        "completion_phases_collapse_to_read" => {
            let value = resolve(
                Some(TimeoutValue::Phases(phases(1.0, 2.0, 3.0, 4.0))),
                &kwargs,
                None,
                supports_httpx_timeout("groq"),
            );
            assert_eq!(value, TimeoutValue::Seconds(seconds(2.0)));
        }
        "completion_phases_without_read_fall_back_to_600" => {
            let declared = TimeoutPhases {
                connect: Some(seconds(1.0)),
                read: None,
                write: Some(seconds(3.0)),
                pool: Some(seconds(4.0)),
            };
            let value = resolve(Some(TimeoutValue::Phases(declared)), &kwargs, None, false);
            assert_eq!(value, TimeoutValue::Seconds(seconds(600.0)));
        }
        "stream_kwargs_beat_every_other_source" => {
            let mut request = request_settings(true);
            request.stream_timeout = Some(seconds(2.0));
            request.timeout = Some(seconds(5.0));
            request.request_timeout = Some(seconds(6.0));
            let mut deployment = no_deployment();
            deployment.stream_timeout = Some(seconds(3.0));
            deployment.timeout = Some(seconds(4.0));
            deployment.request_timeout = Some(seconds(8.0));
            let mut router = router_settings(10.0);
            router.stream_timeout = Some(seconds(7.0));
            assert_eq!(
                stream_timeout(&request, &deployment, &router),
                Some(seconds(2.0))
            );
            assert_eq!(
                non_stream_timeout(&request, &deployment, &router),
                Some(seconds(5.0))
            );
            assert_eq!(
                attempt_timeout(&request, &deployment, &router),
                Some(seconds(2.0))
            );
        }
        "stream_deployment_beats_router" => {
            let request = request_settings(true);
            let mut deployment = no_deployment();
            deployment.stream_timeout = Some(seconds(3.0));
            let mut router = router_settings(10.0);
            router.stream_timeout = Some(seconds(7.0));
            assert_eq!(
                stream_timeout(&request, &deployment, &router),
                Some(seconds(3.0))
            );
            assert_eq!(
                attempt_timeout(&request, &deployment, &router),
                Some(seconds(3.0))
            );
        }
        "stream_router_stream_timeout" => {
            let mut router = router_settings(10.0);
            router.stream_timeout = Some(seconds(7.0));
            assert_eq!(
                stream_timeout(&request_settings(true), &no_deployment(), &router),
                Some(seconds(7.0))
            );
        }
        "stream_falls_back_to_non_stream" => {
            let mut request = request_settings(true);
            request.timeout = Some(seconds(5.0));
            assert_eq!(
                stream_timeout(&request, &no_deployment(), &router_settings(10.0)),
                None
            );
            assert_eq!(
                attempt_timeout(&request, &no_deployment(), &router_settings(10.0)),
                Some(seconds(5.0))
            );
        }
        "non_stream_kwargs_timeout_wins" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            request.request_timeout = Some(seconds(6.0));
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(4.0));
            assert_eq!(
                non_stream_timeout(&request, &deployment, &router_settings(10.0)),
                Some(seconds(5.0))
            );
        }
        "non_stream_kwargs_request_timeout_wins" => {
            let mut request = request_settings(false);
            request.request_timeout = Some(seconds(6.0));
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(4.0));
            assert_eq!(
                non_stream_timeout(&request, &deployment, &router_settings(10.0)),
                Some(seconds(6.0))
            );
        }
        "non_stream_deployment_timeout" => {
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(4.0));
            assert_eq!(
                non_stream_timeout(
                    &request_settings(false),
                    &deployment,
                    &router_settings(10.0)
                ),
                Some(seconds(4.0))
            );
        }
        "non_stream_deployment_request_timeout" => {
            let mut deployment = no_deployment();
            deployment.request_timeout = Some(seconds(8.0));
            assert_eq!(
                non_stream_timeout(
                    &request_settings(false),
                    &deployment,
                    &router_settings(10.0)
                ),
                Some(seconds(8.0))
            );
        }
        "non_stream_router_request_timeout" => {
            let mut router = router_settings(10.0);
            router.request_timeout = Some(seconds(9.0));
            assert_eq!(
                non_stream_timeout(&request_settings(false), &no_deployment(), &router),
                Some(seconds(9.0))
            );
        }
        "non_stream_router_timeout" => {
            assert_eq!(
                non_stream_timeout(
                    &request_settings(false),
                    &no_deployment(),
                    &router_settings(10.0)
                ),
                Some(seconds(10.0))
            );
        }
        "non_stream_global_default" => {
            let sources = RouterTimeoutSources::default();
            let resolved = resolve_router_timeouts(&sources, global_timeout(6000.0, false));
            assert_eq!(
                non_stream_timeout(&request_settings(false), &no_deployment(), &resolved),
                Some(seconds(6000.0))
            );
        }
        "plan_deadline_keeps_request_sources" => {
            let base = Instant::now();
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(4.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &deployment).unwrap();
            assert_eq!(plan.deadline, at(base, 5));
            assert_eq!(plan.attempt, Some(seconds(4.0)));
            assert_eq!(plan.stream, None);
            assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 4));
        }
        "plan_stream_deadline_and_attempt" => {
            let base = Instant::now();
            let mut request = request_settings(true);
            request.stream_timeout = Some(seconds(2.0));
            let mut deployment = no_deployment();
            deployment.stream_timeout = Some(seconds(3.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &deployment).unwrap();
            assert_eq!(plan.deadline, at(base, 2));
            assert_eq!(plan.stream, Some(seconds(3.0)));
            assert_eq!(plan.attempt, Some(seconds(3.0)));
            assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 2));
        }
        other => panic!("unknown behavior {other}"),
    }
}

#[rstest]
#[case::negative_seconds_is_an_error("negative_seconds_is_an_error")]
#[case::nan_seconds_is_an_error("nan_seconds_is_an_error")]
#[case::infinite_seconds_is_an_error("infinite_seconds_is_an_error")]
#[case::huge_seconds_is_an_error("huge_seconds_is_an_error")]
#[case::zero_seconds_is_kept("zero_seconds_is_kept")]
#[case::zero_attempt_expires_immediately("zero_attempt_expires_immediately")]
#[case::attempt_equal_to_deadline("attempt_equal_to_deadline")]
#[case::deadline_reached_at_now("deadline_reached_at_now")]
#[case::cancelled_blocks_attempt("cancelled_blocks_attempt")]
#[case::cancelled_blocks_remaining("cancelled_blocks_remaining")]
#[case::cancelled_expired_is_cancelled("cancelled_expired_is_cancelled")]
#[case::expired_before_the_deadline_is_attempt("expired_before_the_deadline_is_attempt")]
#[case::expired_at_the_deadline_is_deadline("expired_at_the_deadline_is_deadline")]
#[case::global_default_flags("global_default_flags")]
#[case::router_constructor_fields("router_constructor_fields")]
#[case::router_without_timeout_uses_global("router_without_timeout_uses_global")]
fn routing_timeouts_boundaries(#[case] name: &str) {
    let base = Instant::now();
    match name {
        "negative_seconds_is_an_error" => {
            let error = Seconds::new("model_list.0.litellm_params.timeout", -1.0).unwrap_err();
            assert_eq!(
                error,
                TimeoutError::Negative {
                    field: "model_list.0.litellm_params.timeout".to_owned(),
                    seconds: -1.0,
                }
            );
            assert!(
                error
                    .to_string()
                    .contains("model_list.0.litellm_params.timeout")
            );
        }
        "nan_seconds_is_an_error" => {
            let error = Seconds::new("router_settings.timeout", f64::NAN).unwrap_err();
            assert!(matches!(error, TimeoutError::NotFinite { .. }));
            assert!(error.to_string().contains("router_settings.timeout"));
        }
        "infinite_seconds_is_an_error" => {
            let error = Seconds::new("stream_timeout", f64::INFINITY).unwrap_err();
            assert!(matches!(error, TimeoutError::NotFinite { .. }));
        }
        "huge_seconds_is_an_error" => {
            let error = seconds(1e300)
                .duration("router_settings.timeout")
                .unwrap_err();
            assert!(matches!(error, TimeoutError::OutOfRange { .. }));
            assert!(error.to_string().contains("router_settings.timeout"));
        }
        "zero_seconds_is_kept" => {
            let value = resolve(None, &TimeoutKwargs::default(), Some(seconds(0.0)), true);
            assert_eq!(value, TimeoutValue::Seconds(seconds(0.0)));
            assert_eq!(value.transport(), seconds(0.0));
        }
        "zero_attempt_expires_immediately" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(0.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &deployment).unwrap();
            assert_eq!(plan.attempt_deadline(base).unwrap(), base);
            assert_eq!(plan.expired(base), TimeoutError::AttemptExpired);
        }
        "attempt_equal_to_deadline" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let mut deployment = no_deployment();
            deployment.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &deployment).unwrap();
            assert_eq!(plan.attempt_deadline(base).unwrap(), plan.deadline);
            assert_eq!(plan.expired(at(base, 5)), TimeoutError::DeadlineExpired);
        }
        "deadline_reached_at_now" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            assert_eq!(
                plan.attempt_deadline(at(base, 5)),
                Err(TimeoutError::DeadlineExpired)
            );
            assert_eq!(
                plan.remaining(at(base, 5)),
                Err(TimeoutError::DeadlineExpired)
            );
        }
        "cancelled_blocks_attempt" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            plan.cancellation.cancel();
            assert_eq!(plan.attempt_deadline(base), Err(TimeoutError::Cancelled));
        }
        "cancelled_blocks_remaining" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            plan.cancellation.cancel();
            assert_eq!(plan.remaining(base), Err(TimeoutError::Cancelled));
        }
        "cancelled_expired_is_cancelled" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            plan.cancellation.cancel();
            assert_eq!(plan.expired(at(base, 9)), TimeoutError::Cancelled);
        }
        "expired_before_the_deadline_is_attempt" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            let error = plan.expired(at(base, 4));
            assert_eq!(error, TimeoutError::AttemptExpired);
            assert_eq!(error.status_code(), Some(408));
            assert_eq!(error.code(), Some("408"));
        }
        "expired_at_the_deadline_is_deadline" => {
            let mut request = request_settings(false);
            request.timeout = Some(seconds(5.0));
            let plan =
                TimeoutPlan::new(base, &router_settings(10.0), &request, &no_deployment()).unwrap();
            let error = plan.expired(at(base, 5));
            assert_eq!(error, TimeoutError::DeadlineExpired);
            assert_eq!(error.status_code(), Some(408));
        }
        "global_default_flags" => {
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
        "router_constructor_fields" => {
            let sources = RouterTimeoutSources {
                timeout: Some(seconds(7.0)),
                stream_timeout: Some(seconds(3.0)),
                default_timeout: Some(seconds(5.0)),
                default_stream_timeout: Some(seconds(4.0)),
            };
            let resolved = resolve_router_timeouts(&sources, global_timeout(6000.0, true));
            assert_eq!(resolved.timeout, seconds(7.0));
            assert_eq!(resolved.stream_timeout, Some(seconds(3.0)));
            assert_eq!(resolved.request_timeout, Some(seconds(6000.0)));
            assert_eq!(resolved.default_timeout, Some(seconds(5.0)));
            assert_eq!(resolved.default_stream_timeout, Some(seconds(4.0)));
        }
        "router_without_timeout_uses_global" => {
            let resolved = resolve_router_timeouts(
                &RouterTimeoutSources::default(),
                global_timeout(300.0, true),
            );
            assert_eq!(resolved.timeout, seconds(300.0));
            assert_eq!(resolved.request_timeout, None);
            assert_eq!(resolved.default_timeout, None);
        }
        other => panic!("unknown boundary {other}"),
    }
}

fn phases(connect: f64, read: f64, write: f64, pool: f64) -> TimeoutPhases {
    TimeoutPhases {
        connect: Some(seconds(connect)),
        read: Some(seconds(read)),
        write: Some(seconds(write)),
        pool: Some(seconds(pool)),
    }
}

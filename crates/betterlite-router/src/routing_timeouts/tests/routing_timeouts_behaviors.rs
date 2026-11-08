use std::time::Instant;

use rstest::rstest;

use super::{at, global_timeout, no_deployment, request_settings, router_settings, seconds};
use crate::routing_timeouts::completion::{
    TimeoutKwargs, TimeoutPhases, TimeoutValue, resolve, supports_httpx_timeout,
};
use crate::routing_timeouts::plan::{
    RouterTimeoutSources, TimeoutPlan, resolve_router_timeouts, select_timeouts,
};

#[rstest]
#[case::model_beats_kwargs(
    Some(11.0_f64),
    Some(22.0_f64),
    Some(33.0_f64),
    Some(44.0_f64),
    11.0_f64
)]
#[case::kwargs_timeout_beats_request_timeout(
    None,
    Some(22.0_f64),
    Some(33.0_f64),
    Some(44.0_f64),
    22.0_f64
)]
#[case::kwargs_request_timeout(None, None, Some(33.0_f64), Some(44.0_f64), 33.0_f64)]
#[case::configured_global(None, None, None, Some(44.0_f64), 44.0_f64)]
fn completion_precedence(
    #[case] model: Option<f64>,
    #[case] timeout: Option<f64>,
    #[case] request_timeout: Option<f64>,
    #[case] configured: Option<f64>,
    #[case] expected: f64,
) {
    let kwargs = TimeoutKwargs {
        timeout: timeout.map(seconds),
        request_timeout: request_timeout.map(seconds),
    };
    let value = resolve(
        model.map(|value| TimeoutValue::Seconds(seconds(value))),
        &kwargs,
        configured.map(seconds),
        true,
    );
    assert_eq!(value, TimeoutValue::Seconds(seconds(expected)));
}

#[rstest]
#[case::unset_global(6000.0_f64, false, 600.0_f64)]
#[case::explicit_6000(6000.0_f64, true, 6000.0_f64)]
#[case::runtime_300(300.0_f64, false, 300.0_f64)]
fn completion_global_fallback(
    #[case] value: f64,
    #[case] explicitly_set: bool,
    #[case] expected: f64,
) {
    let configured = global_timeout(value, explicitly_set).configured();
    let resolved = resolve(None, &TimeoutKwargs::default(), configured, true);
    assert_eq!(resolved, TimeoutValue::Seconds(seconds(expected)));
}

#[rstest]
#[case::httpx_provider_keeps_phases("openai", Some(2.0_f64), true)]
#[case::other_provider_collapses_to_read("groq", Some(2.5_f64), false)]
#[case::missing_read_falls_back("groq", None, false)]
fn completion_phases(
    #[case] provider: &str,
    #[case] read: Option<f64>,
    #[case] keeps_phases: bool,
) {
    let declared = TimeoutValue::Phases(TimeoutPhases {
        connect: Some(seconds(1.0)),
        read: read.map(seconds),
        write: Some(seconds(3.0)),
        pool: Some(seconds(4.0)),
    });
    let value = resolve(
        Some(declared),
        &TimeoutKwargs::default(),
        None,
        supports_httpx_timeout(provider),
    );
    let fallback = seconds(600.0);
    let expected = if keeps_phases {
        declared
    } else {
        TimeoutValue::Seconds(read.map_or(fallback, seconds))
    };
    assert_eq!(value, expected);
}

#[test]
fn stream_kwargs_beat_every_other_source() {
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
    let selected = select_timeouts(&request, &deployment, &router);
    assert_eq!(selected.stream, Some(seconds(2.0)));
    assert_eq!(selected.non_stream, seconds(5.0));
    assert_eq!(selected.attempt, seconds(2.0));
}

#[test]
fn stream_deployment_beats_router() {
    let request = request_settings(true);
    let mut deployment = no_deployment();
    deployment.stream_timeout = Some(seconds(3.0));
    let mut router = router_settings(10.0);
    router.stream_timeout = Some(seconds(7.0));
    let selected = select_timeouts(&request, &deployment, &router);
    assert_eq!(selected.stream, Some(seconds(3.0)));
    assert_eq!(selected.attempt, seconds(3.0));
}

#[test]
fn stream_router_stream_timeout() {
    let mut router = router_settings(10.0);
    router.stream_timeout = Some(seconds(7.0));
    let selected = select_timeouts(&request_settings(true), &no_deployment(), &router);
    assert_eq!(selected.stream, Some(seconds(7.0)));
    assert_eq!(selected.attempt, seconds(7.0));
}

#[test]
fn stream_falls_back_to_non_stream() {
    let mut request = request_settings(true);
    request.timeout = Some(seconds(5.0));
    let selected = select_timeouts(&request, &no_deployment(), &router_settings(10.0));
    assert_eq!(selected.stream, None);
    assert_eq!(selected.attempt, seconds(5.0));
}

#[test]
fn non_stream_kwargs_timeout_wins() {
    let mut request = request_settings(false);
    request.timeout = Some(seconds(5.0));
    request.request_timeout = Some(seconds(6.0));
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(4.0));
    let selected = select_timeouts(&request, &deployment, &router_settings(10.0));
    assert_eq!(selected.non_stream, seconds(5.0));
}

#[test]
fn non_stream_kwargs_request_timeout_wins() {
    let mut request = request_settings(false);
    request.request_timeout = Some(seconds(6.0));
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(4.0));
    let selected = select_timeouts(&request, &deployment, &router_settings(10.0));
    assert_eq!(selected.non_stream, seconds(6.0));
}

#[test]
fn non_stream_deployment_timeout() {
    let mut deployment = no_deployment();
    deployment.timeout = Some(seconds(4.0));
    let selected = select_timeouts(
        &request_settings(false),
        &deployment,
        &router_settings(10.0),
    );
    assert_eq!(selected.non_stream, seconds(4.0));
}

#[test]
fn non_stream_deployment_request_timeout() {
    let mut deployment = no_deployment();
    deployment.request_timeout = Some(seconds(8.0));
    let selected = select_timeouts(
        &request_settings(false),
        &deployment,
        &router_settings(10.0),
    );
    assert_eq!(selected.non_stream, seconds(8.0));
}

#[test]
fn non_stream_router_request_timeout() {
    let mut router = router_settings(10.0);
    router.request_timeout = Some(seconds(9.0));
    let selected = select_timeouts(&request_settings(false), &no_deployment(), &router);
    assert_eq!(selected.non_stream, seconds(9.0));
}

#[test]
fn non_stream_router_timeout() {
    let selected = select_timeouts(
        &request_settings(false),
        &no_deployment(),
        &router_settings(10.0),
    );
    assert_eq!(selected.non_stream, seconds(10.0));
}

#[test]
fn non_stream_global_default() {
    let resolved = resolve_router_timeouts(
        &RouterTimeoutSources::default(),
        global_timeout(6000.0, false),
    );
    let selected = select_timeouts(&request_settings(false), &no_deployment(), &resolved);
    assert_eq!(selected.non_stream, seconds(6000.0));
}

#[test]
fn plan_deadline_ignores_deployment_sources() {
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
    assert_eq!(plan.stream, None);
    assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 4));
}

#[test]
fn plan_stream_timeout_below_deadline() {
    let base = Instant::now();
    let mut deployment = no_deployment();
    deployment.stream_timeout = Some(seconds(3.0));
    let plan = TimeoutPlan::new(
        base,
        &router_settings(10.0),
        &request_settings(true),
        &deployment,
    )
    .unwrap();
    assert_eq!(plan.deadline, at(base, 10));
    assert_eq!(plan.stream, Some(seconds(3.0)));
    assert_eq!(plan.attempt, seconds(3.0));
    assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 3));
}

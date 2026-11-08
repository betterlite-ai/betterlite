#![cfg(test)]

use std::time::{Duration, Instant};

use betterlite_config::sections::routing_timeouts::{
    DeploymentTimeouts as ConfigDeployment, GlobalTimeout, RouterTimeouts as ConfigRouter, Settings,
};
use betterlite_router::routing_timeouts::error::TimeoutError;
use betterlite_router::routing_timeouts::plan::RequestTimeouts;
use betterlite_router::routing_timeouts::seconds::Seconds;

use super::{RoutingTimeoutError, plan, resolve};

fn section(
    global: (f64, bool),
    router: ConfigRouter,
    deployments: Vec<ConfigDeployment>,
) -> Settings {
    Settings {
        global: GlobalTimeout {
            seconds: global.0,
            explicitly_set: global.1,
        },
        router,
        deployments,
    }
}

fn at(base: Instant, seconds: u64) -> Instant {
    base.checked_add(Duration::from_secs(seconds)).unwrap()
}

#[test]
fn resolve_maps_every_level() {
    let deployment = ConfigDeployment {
        timeout: Some(11.0_f64),
        stream_timeout: Some(12.0_f64),
        request_timeout: Some(13.0_f64),
    };
    let router = ConfigRouter {
        timeout: Some(7.0_f64),
        stream_timeout: Some(3.0_f64),
        default_timeout: Some(5.0_f64),
        default_stream_timeout: Some(4.0_f64),
    };
    let resolved = resolve(&section((300.0_f64, true), router, vec![deployment])).unwrap();
    assert_eq!(resolved.global.seconds, Seconds(300.0_f64));
    assert!(resolved.global.explicitly_set);
    assert_eq!(resolved.router.timeout, Seconds(7.0_f64));
    assert_eq!(resolved.router.stream_timeout, Some(Seconds(3.0_f64)));
    assert_eq!(resolved.router.request_timeout, Some(Seconds(300.0_f64)));
    assert_eq!(resolved.router.default_timeout, Some(Seconds(5.0_f64)));
    assert_eq!(
        resolved.router.default_stream_timeout,
        Some(Seconds(4.0_f64))
    );
    assert_eq!(resolved.deployments[0].timeout, Some(Seconds(11.0_f64)));
    assert_eq!(
        resolved.deployments[0].stream_timeout,
        Some(Seconds(12.0_f64))
    );
    assert_eq!(
        resolved.deployments[0].request_timeout,
        Some(Seconds(13.0_f64))
    );
}

#[test]
fn resolve_without_router_timeout_uses_the_global() {
    let resolved = resolve(&section(
        (300.0_f64, true),
        ConfigRouter::default(),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(resolved.router.timeout, Seconds(300.0_f64));
    assert_eq!(resolved.router.request_timeout, None);
    assert_eq!(resolved.router.default_timeout, None);
}

#[test]
fn resolve_rejects_a_negative_global() {
    let error = resolve(&section(
        (-1.0_f64, true),
        ConfigRouter::default(),
        Vec::new(),
    ))
    .unwrap_err();
    let RoutingTimeoutError::Timeout { source } = error else {
        panic!("expected the timeout error");
    };
    assert!(matches!(source, TimeoutError::Negative { .. }));
    assert!(
        source
            .to_string()
            .contains("litellm_settings.request_timeout")
    );
}

#[test]
fn resolve_rejects_a_negative_deployment_value() {
    let deployment = ConfigDeployment {
        timeout: Some(-0.5_f64),
        stream_timeout: None,
        request_timeout: None,
    };
    let error = resolve(&section(
        (300.0_f64, false),
        ConfigRouter::default(),
        vec![deployment],
    ))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("model_list.0.litellm_params.timeout")
    );
}

#[test]
fn plan_keeps_the_request_deadline_apart_from_the_attempt() {
    let deployment = ConfigDeployment {
        timeout: Some(4.0_f64),
        stream_timeout: None,
        request_timeout: None,
    };
    let router = ConfigRouter {
        timeout: Some(5.0_f64),
        stream_timeout: None,
        default_timeout: None,
        default_stream_timeout: None,
    };
    let settings = resolve(&section((6000.0_f64, false), router, vec![deployment])).unwrap();
    let base = Instant::now();
    let request = RequestTimeouts {
        stream: false,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    };
    let plan = plan(&settings, &request, 0, base).unwrap();
    assert_eq!(plan.deadline, at(base, 5));
    assert_eq!(plan.attempt, Some(Seconds(4.0_f64)));
    assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 4));
}

#[test]
fn plan_rejects_an_unknown_deployment() {
    let settings = resolve(&section(
        (6000.0_f64, false),
        ConfigRouter::default(),
        Vec::new(),
    ))
    .unwrap();
    let error = plan(&settings, &RequestTimeouts::default(), 3, Instant::now()).unwrap_err();
    assert!(matches!(
        error,
        RoutingTimeoutError::UnknownDeployment { index: 3 }
    ));
}

#[test]
fn cancelled_plan_stops_the_attempt() {
    let deployment = ConfigDeployment {
        timeout: Some(4.0_f64),
        stream_timeout: None,
        request_timeout: None,
    };
    let settings = resolve(&section(
        (6000.0_f64, false),
        ConfigRouter::default(),
        vec![deployment],
    ))
    .unwrap();
    let base = Instant::now();
    let plan = plan(&settings, &RequestTimeouts::default(), 0, base).unwrap();
    plan.cancellation.cancel();
    assert_eq!(plan.attempt_deadline(base), Err(TimeoutError::Cancelled));
}

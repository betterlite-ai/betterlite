#![cfg(test)]

use std::time::{Duration, Instant};

use betterlite_config::sections::routing_timeouts::{
    DeploymentTimeouts as ConfigDeployment, GlobalTimeout, RouterTimeouts as ConfigRouter, Settings,
};
use betterlite_router::routing_timeouts::error::TimeoutError;
use betterlite_router::routing_timeouts::plan::RequestTimeouts;

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
        timeout: Some(11.0),
        stream_timeout: Some(12.0),
        request_timeout: Some(13.0),
    };
    let router = ConfigRouter {
        timeout: Some(7.0),
        stream_timeout: Some(3.0),
        default_timeout: Some(5.0),
        default_stream_timeout: Some(4.0),
    };
    let resolved = resolve(&section((300.0, true), router, vec![deployment])).unwrap();
    assert_eq!(resolved.global.seconds.0, 300.0);
    assert!(resolved.global.explicitly_set);
    assert_eq!(resolved.router.timeout.0, 7.0);
    assert_eq!(resolved.router.stream_timeout.unwrap().0, 3.0);
    assert_eq!(resolved.router.request_timeout.unwrap().0, 300.0);
    assert_eq!(resolved.router.default_timeout.unwrap().0, 5.0);
    assert_eq!(resolved.router.default_stream_timeout.unwrap().0, 4.0);
    assert_eq!(resolved.deployments[0].timeout.unwrap().0, 11.0);
    assert_eq!(resolved.deployments[0].stream_timeout.unwrap().0, 12.0);
    assert_eq!(resolved.deployments[0].request_timeout.unwrap().0, 13.0);
}

#[test]
fn resolve_without_router_timeout_uses_the_global() {
    let resolved = resolve(&section((300.0, true), ConfigRouter::default(), Vec::new())).unwrap();
    assert_eq!(resolved.router.timeout.0, 300.0);
    assert_eq!(resolved.router.request_timeout, None);
    assert_eq!(resolved.router.default_timeout, None);
}

#[test]
fn resolve_rejects_a_negative_global() {
    let error = resolve(&section((-1.0, true), ConfigRouter::default(), Vec::new())).unwrap_err();
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
        timeout: Some(-0.5),
        stream_timeout: None,
        request_timeout: None,
    };
    let error = resolve(&section(
        (300.0, false),
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
        timeout: Some(4.0),
        stream_timeout: None,
        request_timeout: None,
    };
    let settings = resolve(&section(
        (6000.0, false),
        ConfigRouter::default(),
        vec![deployment],
    ))
    .unwrap();
    let base = Instant::now();
    let request = RequestTimeouts {
        stream: false,
        timeout: Some(betterlite_router::routing_timeouts::seconds::Seconds(5.0)),
        request_timeout: None,
        stream_timeout: None,
    };
    let plan = plan(&settings, &request, 0, base).unwrap();
    assert_eq!(plan.deadline, at(base, 5));
    assert_eq!(plan.attempt.unwrap().0, 4.0);
    assert_eq!(plan.attempt_deadline(base).unwrap(), at(base, 4));
}

#[test]
fn plan_rejects_an_unknown_deployment() {
    let settings = resolve(&section(
        (6000.0, false),
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
        timeout: Some(4.0),
        stream_timeout: None,
        request_timeout: None,
    };
    let settings = resolve(&section(
        (6000.0, false),
        ConfigRouter::default(),
        vec![deployment],
    ))
    .unwrap();
    let base = Instant::now();
    let plan = plan(&settings, &RequestTimeouts::default(), 0, base).unwrap();
    plan.cancellation.cancel();
    assert_eq!(plan.attempt_deadline(base), Err(TimeoutError::Cancelled));
}

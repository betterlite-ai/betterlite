use std::io;

use redis::{ErrorKind, RedisError, ServerErrorKind};

use super::super::error::{Observed, StateStoreError};
use super::super::test_scenario::{current, label, leaked};
use crate::redis_failure_policy::classify::FailureClass;
use crate::redis_failure_policy::run::RedisCallError as _;
use crate::routing_value_store::{RouteValue, StoreRead};

pub(crate) fn run(name: &str) {
    match name {
        "redis_errors_map_to_failure_classes" => redis_errors_map_to_failure_classes(),
        "scenario_helpers_report_outcomes" => scenario_helpers_report_outcomes(),
        other => panic!("unknown case {other}"),
    }
}

fn redis_errors_map_to_failure_classes() {
    let cases: [(RedisError, Option<FailureClass>); 6] = [
        (
            RedisError::from(io::Error::from(io::ErrorKind::TimedOut)),
            Some(FailureClass::Timeout),
        ),
        (
            RedisError::from((
                ErrorKind::Server(ServerErrorKind::BusyLoading),
                "the dataset is loading",
            )),
            Some(FailureClass::BusyLoading),
        ),
        (
            RedisError::from((
                ErrorKind::Server(ServerErrorKind::ClusterDown),
                "the cluster is down",
            )),
            Some(FailureClass::ClusterDown),
        ),
        (
            RedisError::from(io::Error::from(io::ErrorKind::BrokenPipe)),
            Some(FailureClass::Connection),
        ),
        (
            RedisError::from(io::Error::from(io::ErrorKind::Other)),
            Some(FailureClass::Connection),
        ),
        (RedisError::from((ErrorKind::Parse, "a bad reply")), None),
    ];
    for (source, expected) in cases {
        let error = StateStoreError::from(("get", source));
        assert_eq!(
            error.failure().map(|failure| failure.class),
            expected,
            "{error:?}"
        );
    }
}

fn scenario_helpers_report_outcomes() {
    let open = StateStoreError::CircuitOpen {
        operation: "get".to_owned(),
    };
    assert_eq!(label(&open), "circuit_open");
    assert_eq!(
        label(&StateStoreError::InvalidTtl { seconds: 0.0_f64 }),
        "invalid_ttl"
    );
    assert_eq!(label(&StateStoreError::NotConfigured), "unavailable");
    assert_eq!(current(StoreRead::Missing), None);
    assert_eq!(
        current(StoreRead::Value(RouteValue::Counter(3))),
        Some(RouteValue::Counter(3))
    );
    let failing = || {
        Err(StateStoreError::Conflict {
            key: "route-secret".to_owned(),
        })
    };
    assert!(leaked(failing(), Some("secret")));
    assert!(!leaked(failing(), Some("absent")));
    assert!(!leaked(failing(), None));
    let observed = Observed {
        value: RouteValue::Counter(1),
        warning: None,
    };
    assert!(!leaked(Ok(observed), Some("secret")));
}

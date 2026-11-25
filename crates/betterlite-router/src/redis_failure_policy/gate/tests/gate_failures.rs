#![cfg(test)]

use std::future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate as _;
use tokio::time::timeout;
use tracing::Level;

use super::support::{
    Broken, FakeClock, OuterIo, OuterRedis, bare_busy_loading_error, bare_cluster_down_error,
    bare_command_error, bare_connection_error, bare_master_down_error, bare_max_clients_error,
    bare_no_auth_error, bare_timeout_error, busy_loading_error, capture_logs,
    chained_timeout_error, cluster_down_error, command_error, connect_timeout_error,
    connection_error, decode_error, described_connection_error, extension_error, gate_with,
    master_down_error, max_clients_error, no_auth_error, reply_error, settings, status,
    timeout_error,
};
use crate::redis_failure_policy::classify::FailureClass;
use crate::redis_failure_policy::gate::failure_class::classify;

#[test]
fn a_connection_error_counts_as_a_hard_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = connection_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
    assert_eq!(observed.timeout_count, 0);
}

#[test]
fn a_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_bare_redis_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_bare_redis_connection_error_counts_as_a_hard_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_connection_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
    assert_eq!(observed.timeout_count, 0);
}

#[test]
fn a_refused_connection_classifies_as_a_connection_failure() {
    let error = OuterIo {
        source: io::Error::new(io::ErrorKind::ConnectionRefused, "Connection refused"),
    };
    let (class, causes) = classify(&error).expect("an io error classifies");
    assert_eq!(class, FailureClass::Connection);
    assert!(causes.is_empty());
}

#[test]
fn a_server_reply_classifies_as_a_command_failure() {
    let error = bare_command_error();
    let (class, _causes) = classify(&error).expect("the reply classifies");
    assert_eq!(class, FailureClass::Command);
}

#[test]
fn an_extension_reply_classifies_as_a_command_failure() {
    let error = OuterRedis {
        source: extension_error(),
    };
    let (class, _causes) = classify(&error).expect("the reply classifies");
    assert_eq!(class, FailureClass::Command);
}

#[test]
fn a_bare_redis_connection_error_classifies_as_a_connection_failure() {
    let error = bare_connection_error();
    let (class, _causes) = classify(&error).expect("the error classifies");
    assert_eq!(class, FailureClass::Connection);
}

#[test]
fn a_bare_no_auth_reply_classifies_as_a_connection_failure() {
    let error = bare_no_auth_error();
    let (class, _causes) = classify(&error).expect("the reply classifies");
    assert_eq!(class, FailureClass::Connection);
}

#[test]
fn a_connection_error_chained_from_a_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = chained_timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_loading_server_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = busy_loading_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_cluster_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = cluster_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_loading_server_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_busy_loading_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_cluster_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_cluster_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_master_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_master_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_master_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = master_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_no_auth_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_no_auth_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_no_auth_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = no_auth_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_max_clients_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_max_clients_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_max_clients_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = max_clients_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_wrong_pass_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = reply_error(b"-WRONGPASS invalid username-password pair or user is disabled.\r\n");
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 1);
}

#[test]
fn a_bare_invalid_password_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = reply_error(b"-ERR invalid password\r\n");
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 1);
}

#[test]
fn a_bare_auth_without_password_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = reply_error(
        b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n",
    );
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 1);
}

#[test]
fn a_bare_client_auth_without_password_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = reply_error(b"-ERR Client sent AUTH, but no password is set\r\n");
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 1);
}

#[test]
fn a_bare_ldap_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = reply_error(b"-ERR problem with LDAP service\r\n");
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 1);
}

#[tokio::test]
async fn a_connect_timeout_counts_as_a_timeout() {
    let elapsed = timeout(Duration::ZERO, future::pending::<()>())
        .await
        .expect_err("the timeout expires");
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = connect_timeout_error(elapsed);
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_command_error_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = command_error();
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 0);
    gate.enter("async_get_cache")
        .expect("the breaker stays closed");
}

#[test]
fn a_decode_error_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = decode_error();
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 0);
    gate.enter("async_get_cache")
        .expect("the breaker stays closed");
}

#[test]
fn an_error_without_a_redis_cause_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    gate.swallowed(&admission, &Broken);
    gate.exit(admission);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "an unknown error leaves no marker, so the call still succeeds"
    );
}

#[test]
fn a_redis_error_without_a_source_records_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = described_connection_error();
    gate.record_failure(&error);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "the source walk skips the redis error, so a description-only error has no class"
    );
}

#[test]
fn swallowed_counts_a_health_failure_too() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let error = connection_error();
    gate.swallowed(&admission, &error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn swallowed_counts_a_no_auth_reply_too() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let error = no_auth_error();
    gate.swallowed(&admission, &error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn swallowed_counts_a_bare_master_down_reply_too() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let error = bare_master_down_error();
    gate.swallowed(&admission, &error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_timeout_failure_logs_an_error_line() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(100), &clock);
    let error = timeout_error();
    let lines = capture_logs(|| gate.record_failure(&error));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].level, Level::ERROR);
    assert_eq!(lines[0].message, "Redis operation failed");
}

#[test]
fn a_connection_failure_logs_every_line_at_error() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(100), &clock);
    let error = connection_error();
    let lines = capture_logs(|| {
        gate.record_failure(&error);
        gate.record_failure(&error);
    });
    let errors: Vec<&str> = lines
        .iter()
        .filter(|line| line.level == Level::ERROR)
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(
        errors,
        ["Redis operation failed", "Redis operation failed"],
        "the timeout throttle leaves hard failures alone"
    );
}

#[test]
fn a_timeout_within_the_interval_logs_at_debug() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(100), &clock);
    let error = timeout_error();
    let lines = capture_logs(|| {
        gate.record_failure(&error);
        gate.record_failure(&error);
    });
    let errors = lines
        .iter()
        .filter(|line| line.level == Level::ERROR)
        .count();
    assert_eq!(
        errors, 1,
        "the second line inside the interval is suppressed"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.level == Level::DEBUG
                && line.message.as_str() == "Redis operation failed"),
        "the suppressed line still goes out at DEBUG"
    );
}

#[test]
fn the_timeout_summary_counts_the_suppressed_lines() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(100), &clock);
    let error = timeout_error();
    let lines = capture_logs(|| {
        gate.record_failure(&error);
        gate.record_failure(&error);
        clock.advance(60_000);
        gate.record_failure(&error);
    });
    let errors: Vec<&str> = lines
        .iter()
        .filter(|line| line.level == Level::ERROR)
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0], "Redis operation failed");
    assert_eq!(
        errors[1],
        "Redis operation failed (1 more Redis timeouts since the previous Redis timeout line were logged at DEBUG)"
    );
}

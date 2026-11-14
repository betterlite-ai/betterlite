use std::time::Duration;

use tokio::time::sleep;

use crate::health_probe_settings::timeout::{HealthCheckTimeout, run_with_timeout};

#[tokio::test(start_paused = true)]
async fn a_task_inside_the_limit_returns_its_value() {
    let outcome = run_with_timeout(sleep(Duration::from_millis(500)), Duration::from_secs(1)).await;
    assert_eq!(outcome, Ok(()));
}

#[tokio::test(start_paused = true)]
async fn a_task_over_the_limit_times_out() {
    let outcome = run_with_timeout(sleep(Duration::from_secs(2)), Duration::from_secs(1)).await;
    assert_eq!(outcome, Err(HealthCheckTimeout));
}

#[tokio::test(start_paused = true)]
async fn a_timeout_leaves_a_sibling_task_running() {
    let slow = run_with_timeout(sleep(Duration::from_secs(10)), Duration::from_secs(1));
    let fast = run_with_timeout(sleep(Duration::from_secs(2)), Duration::from_secs(5));
    let (timed_out, finished) = tokio::join!(slow, fast);
    assert_eq!(timed_out, Err(HealthCheckTimeout));
    assert_eq!(finished, Ok(()));
}

#[test]
fn the_timeout_error_carries_litellms_message() {
    assert_eq!(
        HealthCheckTimeout.to_string(),
        "Health check timeout exceeded"
    );
}

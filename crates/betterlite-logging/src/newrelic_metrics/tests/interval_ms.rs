use super::support::{Fixture, Reply, ScriptedClock, record_at};
use crate::newrelic_metrics::flush::{async_send_batch, flush_queue};
use crate::newrelic_metrics::queue::push_record;

#[tokio::test]
async fn interval_ms_measures_the_real_window() {
    let mut fixture = Fixture::new();
    fixture.clock = ScriptedClock::new(&[1_700_000_000.0_f64, 1_700_000_001.000_9_f64]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let payloads = fixture.transport.payloads();
    let common = &payloads[0][0].common;
    assert_eq!(common.timestamp, 1_700_000_000_000);
    assert_eq!(common.interval_ms, 1_000);
}

#[tokio::test]
async fn interval_ms_never_falls_below_one() {
    let mut fixture = Fixture::new();
    fixture.clock = ScriptedClock::new(&[1_700_000_000.0_f64, 1_700_000_000.000_4_f64]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let payloads = fixture.transport.payloads();
    assert_eq!(payloads[0][0].common.interval_ms, 1);
}

#[tokio::test]
async fn interval_ms_starts_at_the_previous_flush_time() {
    let mut fixture = Fixture::new_with_replies(&[Reply::Status(202), Reply::Status(202)]);
    fixture.clock = ScriptedClock::new(&[
        1_700_000_000.0_f64,
        1_700_000_003.0_f64,
        1_700_000_004.0_f64,
        1_700_000_008.0_f64,
        1_700_000_009.0_f64,
    ]);
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    flush_queue(&logger).await;
    push_record(&logger, record_at(1));
    flush_queue(&logger).await;
    let payloads = fixture.transport.payloads();
    assert_eq!(payloads[0][0].common.timestamp, 1_700_000_000_000);
    assert_eq!(payloads[0][0].common.interval_ms, 4_000);
    assert_eq!(payloads[1][0].common.timestamp, 1_700_000_003_000);
    assert_eq!(payloads[1][0].common.interval_ms, 6_000);
}

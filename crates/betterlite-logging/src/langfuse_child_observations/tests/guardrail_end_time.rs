#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rstest::rstest;

use super::guardrails::log_guardrail_spans;
use super::support::child;
use super::support::{generation, payload, provider, sdk_tracer, spans};

const START_ONLY: &str = r#"[{"guardrail_name":"pii-detector","start_time":1.5}]"#;
const END_ONLY: &str = r#"[{"guardrail_name":"pii-detector","end_time":2.25}]"#;

fn nanosecond_time(nanos: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_nanos(nanos))
        .expect("a representable time")
}

#[rstest]
#[case::with_a_start_time(START_ONLY, Some(1_500_000_000), None)]
#[case::with_an_end_time(END_ONLY, None, Some(2_250_000_000))]
fn guardrail_end_time_closes_the_span_at_the_guardrail_time(
    #[case] guardrails: &str,
    #[case] start_nanos: Option<u64>,
    #[case] end_nanos: Option<u64>,
) {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let standard = payload(Some(guardrails)).expect("the payload parses");
    log_guardrail_spans(&observations, Some(&standard));
    let finished = spans(&exporter);
    let data = finished.first().expect("a span");
    if let Some(nanos) = start_nanos {
        assert_eq!(data.start_time, nanosecond_time(nanos));
    }
    if let Some(nanos) = end_nanos {
        assert_eq!(data.end_time, nanosecond_time(nanos));
    }
    if start_nanos.is_none() {
        assert_ne!(data.start_time, UNIX_EPOCH);
    }
    if end_nanos.is_none() {
        assert_ne!(data.end_time, UNIX_EPOCH);
    }
}

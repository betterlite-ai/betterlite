use betterlite_logging::otel::logger::call_context::CallContext;
use rust_decimal::Decimal;

use crate::otel::support::{inference, stream_event, zero_completion_start};

#[test]
fn call_context_measures_the_first_chunk_of_a_streaming_call() {
    let call = CallContext::from_event(&stream_event(inference()), None).expect("call context");
    assert_eq!(
        call.event.time_to_first_chunk_seconds,
        Some(Decimal::new(5, 1))
    );
}

#[test]
fn call_context_ignores_a_zero_first_chunk_stamp() {
    let call = CallContext::from_event(&stream_event(zero_completion_start(inference())), None)
        .expect("call context");
    assert_eq!(call.event.time_to_first_chunk_seconds, None);
}

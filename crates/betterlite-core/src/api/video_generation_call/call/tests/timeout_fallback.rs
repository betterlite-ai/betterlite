use std::time::Duration;

use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, REQUEST_TIMEOUT, failed, handled, handles, request};

#[test]
fn the_declared_default_timeout_reaches_the_handler() {
    let mut bench = Bench::new(Outcome::Answers);
    bench.run(request()).expect("the call answers");
    assert_eq!(handled(&bench.journal).timeout, Duration::from_secs(600));
}

#[test]
fn a_missing_timeout_takes_the_request_timeout() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.timeout = None;
    bench.run(call).expect("the call answers");
    assert_eq!(handled(&bench.journal).timeout, REQUEST_TIMEOUT);
}

#[test]
fn a_zero_timeout_takes_the_request_timeout() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.timeout = Some(0.0_f64);
    bench.run(call).expect("the call answers");
    assert_eq!(handled(&bench.journal).timeout, REQUEST_TIMEOUT);
}

#[test]
fn an_unusable_timeout_is_refused_before_the_handler() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.timeout = Some(-1.0_f64);
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "the request timeout of -1 seconds is not a usable duration"
    );
    let ErrorClass::ApiConnection(context) = &raised.class else {
        panic!(
            "an unusable timeout is a value error, got {:?}",
            raised.class
        );
    };
    assert_eq!(
        context.message,
        "APIConnectionError: OpenAIException - the request timeout of -1 seconds is not a usable duration"
    );
    assert_eq!(context.model.as_deref(), Some("sora-2"));
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(handles(&bench.journal).is_empty());
}

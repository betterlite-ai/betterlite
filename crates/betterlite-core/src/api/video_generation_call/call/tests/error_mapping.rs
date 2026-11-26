use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::support::{Bench, Outcome, failed, handles, request};

#[test]
fn a_sync_handler_failure_is_mapped_with_the_resolved_model() {
    let mut bench = Bench::new(Outcome::FieldType);
    let raised = failed(bench.run(request()));
    assert_eq!(raised.error.to_string(), "size must be a string");
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!(
            "a value error the handler names answers 400, got {:?}",
            raised.class
        );
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.message, "size must be a string");
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[test]
fn an_option_the_declared_type_cannot_hold_is_a_400_naming_the_field() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs
        .insert("seconds".to_owned(), Json::Number(Number::from(8_u64)));
    let raised = failed(bench.run(call));
    assert_eq!(raised.error.to_string(), "seconds must be a string");
    let ErrorClass::BadRequest(details) = &raised.class else {
        panic!("a wrong option type answers 400, got {:?}", raised.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.message, "seconds must be a string");
    assert_eq!(details.model.as_deref(), Some("sora-2"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert!(handles(&bench.journal).is_empty());
    assert!(bench.caller.providers.config.calls().is_empty());
    assert!(bench.fixture.state.call_type.is_empty());
}

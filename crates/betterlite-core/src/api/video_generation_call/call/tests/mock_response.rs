use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    Bench, MOCK_JSON, Outcome, failed, handles, lookups, object_json, request, text,
};

#[test]
fn a_mock_string_answers_before_any_lookup_or_logging() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs
        .insert("mock_response".to_owned(), text(MOCK_JSON));
    let object = bench.run(call).expect("the mock answers");
    assert_eq!(object.id, "mock-1");
    assert_eq!(object.status, "completed");
    assert!(lookups(&bench.journal).is_empty(), "no config was selected");
    assert!(handles(&bench.journal).is_empty(), "the handler never ran");
    assert!(
        bench.caller.providers.config.calls().is_empty(),
        "the options were never mapped"
    );
    assert_eq!(bench.fixture.state.model, "video-model");
    assert!(bench.fixture.state.call_type.is_empty());
    assert!(bench.fixture.state.optional_params.is_empty());
}

#[test]
fn a_mock_map_answers_with_the_mock_object() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs
        .insert("mock_response".to_owned(), object_json("mock-2"));
    let object = bench.run(call).expect("the mock answers");
    assert_eq!(object.id, "mock-2");
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn a_mock_answers_even_when_async_call_is_true() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs
        .insert("async_call".to_owned(), Json::Bool(true));
    call.kwargs
        .insert("mock_response".to_owned(), object_json("mock-3"));
    let object = bench.run(call).expect("the mock answers without a handler");
    assert_eq!(object.id, "mock-3");
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn a_mock_wins_over_an_unknown_provider() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.custom_llm_provider = Some("not-a-provider".to_owned());
    call.kwargs
        .insert("mock_response".to_owned(), text(MOCK_JSON));
    let object = bench.run(call).expect("the mock answers before the lookup");
    assert_eq!(object.id, "mock-1");
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn a_mock_that_is_not_json_is_an_api_connection_error() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs
        .insert("mock_response".to_owned(), text("{not json"));
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "the mock response is not valid JSON"
    );
    let ErrorClass::ApiConnection(context) = &raised.class else {
        panic!("an invalid mock is an api connection error");
    };
    assert_eq!(
        context.message,
        "Exception - the mock response is not valid JSON"
    );
    assert_eq!(context.model.as_deref(), Some("sora-2"));
    assert_eq!(context.provider.as_deref(), Some(""));
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn an_empty_model_in_a_bad_mock_is_named_as_the_default() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.model = Some(String::new());
    call.kwargs
        .insert("mock_response".to_owned(), text("{not json"));
    let raised = failed(bench.run(call));
    let ErrorClass::ApiConnection(context) = &raised.class else {
        panic!(
            "an invalid mock is an api connection error, got {:?}",
            raised.class
        );
    };
    assert_eq!(context.model.as_deref(), Some("sora-2"));
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn a_mock_without_the_required_fields_is_refused() {
    let mut bench = Bench::new(Outcome::Answers);
    let mut call = request();
    call.kwargs.insert(
        "mock_response".to_owned(),
        text(r#"{"object":"video","status":"completed"}"#),
    );
    let raised = failed(bench.run(call));
    assert_eq!(
        raised.error.to_string(),
        "the mock response does not match a video object"
    );
    assert!(matches!(raised.class, ErrorClass::ApiConnection(_)));
    assert!(handles(&bench.journal).is_empty());
}

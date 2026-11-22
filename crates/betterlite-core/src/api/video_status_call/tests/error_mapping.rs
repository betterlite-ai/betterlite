use betterlite_error::error::{ErrorClass, ErrorContext};

use super::support::{
    Behavior, Bench, Environment, Journal, Mode, Outcome, PROVIDER, Production, VIDEO_ID, Wiring,
    async_call, call, failed, request,
};
use crate::api::video_status_call::error::VideoStatusCallError;

fn mapped_class() -> ErrorClass {
    ErrorClass::ApiConnection(Box::new(ErrorContext {
        message: "the provider refused".to_owned(),
        model: Some(String::new()),
        provider: Some(PROVIDER.to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

fn failing_outcome() -> Outcome {
    Outcome::fails(mapped_class(), "the provider refused")
}

#[test]
fn error_mapping_status_handler_error_is_not_mapped_again() {
    let mut bench = Bench::new(failing_outcome());
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    let VideoStatusCallError::Mapped { class, message } = &error else {
        panic!("the handler error stays mapped");
    };
    assert_eq!(class, &mapped_class());
    assert_eq!(message, "the provider refused");
    assert_eq!(error.to_string(), "the provider refused");
}

#[tokio::test]
async fn error_mapping_status_async_wrapper_keeps_the_mapped_class() {
    let mut bench = Bench::new(failing_outcome());
    let error = async_call(&mut bench, request(VIDEO_ID))
        .await
        .expect_err("the handler fails");
    let VideoStatusCallError::Mapped { class, message } = &error else {
        panic!("the handler error stays mapped");
    };
    assert_eq!(class, &mapped_class());
    assert_eq!(message, "the provider refused");
}

#[test]
fn error_mapping_status_transport_failure_keeps_the_provider_status() {
    let journal = Journal::new();
    let wiring = Wiring::new(&journal);
    let environment = Environment::new();
    let prepost = environment.prepost();
    let mut production = Production::new(&journal, Behavior::Found, Mode::Fails, &wiring, &prepost);
    let work =
        production
            .caller
            .video_status(request(VIDEO_ID), None, &mut production.fixture.state);
    let error = failed(work);
    assert_eq!(error.to_string(), "the provider refused");
    let VideoStatusCallError::Mapped {
        class: ErrorClass::BadRequest(details),
        ..
    } = &error
    else {
        panic!("a provider 400 is a bad request");
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "gemini: Vertex AI rejected the request body"
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(PROVIDER));
}

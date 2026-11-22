use betterlite_error::error::{ErrorClass, ErrorContext};

use super::support::{
    Bench, Fixture, Journal, Outcome, RecordingConfig, VIDEO_ID, WIRE_BODY, Wire, async_call, call,
    failed, request, transport_caller,
};
use crate::api::video_content_call::VideoContentCallError;
use crate::api::videos_http_transport::client::{ClientKind, SuppliedClient};

fn mapped_class() -> ErrorClass {
    ErrorClass::ApiConnection(Box::new(ErrorContext {
        message: "the provider refused".to_owned(),
        model: Some(String::new()),
        provider: Some("gemini".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

fn failed_outcome() -> Outcome {
    Outcome::Fails {
        class: mapped_class(),
        message: "the provider refused".to_owned(),
    }
}

#[test]
fn error_mapping_content_handler_error_is_not_mapped_again() {
    let mut bench = Bench::new(failed_outcome());
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    let VideoContentCallError::Mapped { class, message } = &error else {
        panic!("the handler error stays mapped");
    };
    assert_eq!(class, &mapped_class());
    assert_eq!(message, "the provider refused");
    assert_eq!(error.to_string(), "the provider refused");
}

#[tokio::test]
async fn error_mapping_content_async_wrapper_keeps_the_mapped_class() {
    let mut bench = Bench::new(failed_outcome());
    let error = async_call(&mut bench, request(VIDEO_ID))
        .await
        .expect_err("the handler fails");
    let VideoContentCallError::Mapped { class, message } = &error else {
        panic!("the handler error stays mapped");
    };
    assert_eq!(class, &mapped_class());
    assert_eq!(message, "the provider refused");
}

#[test]
fn error_mapping_content_transport_failure_keeps_the_provider_status() {
    let journal = Journal::new();
    let caller = transport_caller(&journal, RecordingConfig::failing());
    let mut fixture = Fixture::new();
    let wire = Wire::new();
    let work = caller.video_content(
        request(VIDEO_ID),
        Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &wire,
        }),
        &mut fixture.state,
    );
    let error = failed(work);
    assert_eq!(error.to_string(), "the transform refused");
    let VideoContentCallError::Mapped {
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
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(wire.seen().len(), 1);
    assert_eq!(
        caller.providers.config.replies(),
        vec![(200_u16, WIRE_BODY.as_bytes().to_vec())]
    );
}

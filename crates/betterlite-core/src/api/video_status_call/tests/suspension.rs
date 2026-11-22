use betterlite_error::error::ErrorClass;

use super::support::{
    Behavior, FakeProviders, Fixture, Journal, Mode, NeverAnswers, REQUEST_TIMEOUT, VIDEO_ID,
    failed, redactor, request,
};
use crate::api::video_status_call::VideoStatusCaller;
use crate::api::video_status_call::error::VideoStatusCallError;

#[test]
fn suspension_status_sync_call_on_a_pending_handler_errors() {
    let journal = Journal::new();
    let caller = VideoStatusCaller {
        providers: FakeProviders::new(&journal, Behavior::Found, Mode::Answers),
        handler: NeverAnswers,
        redactor: redactor(),
        request_timeout: REQUEST_TIMEOUT,
    };
    let mut fixture = Fixture::new();
    let error = failed(caller.video_status(request(VIDEO_ID), None, &mut fixture.state));
    assert_eq!(
        error.to_string(),
        "the blocking status call waited on a handler that never answered"
    );
    let VideoStatusCallError::Mapped {
        class: ErrorClass::ApiConnection(context),
        ..
    } = &error
    else {
        panic!("a suspended call is an api connection error");
    };
    assert_eq!(
        context.message,
        "GeminiException - the blocking status call waited on a handler that never answered"
    );
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("gemini"));
}

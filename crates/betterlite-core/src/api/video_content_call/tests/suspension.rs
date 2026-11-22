use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, VIDEO_ID, call, failed, request};
use crate::api::video_content_call::error::VideoContentCallError;

#[test]
fn suspension_sync_call_on_a_pending_handler_errors() {
    let mut bench = Bench::new(Outcome::NeverAnswers);
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(
        error.to_string(),
        "the blocking content call waited on a handler that never answered"
    );
    let VideoContentCallError::Mapped {
        class: ErrorClass::ApiConnection(context),
        ..
    } = &error
    else {
        panic!("a suspended call is an api connection error");
    };
    assert_eq!(
        context.message,
        "GeminiException - the blocking content call waited on a handler that never answered"
    );
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("gemini"));
}

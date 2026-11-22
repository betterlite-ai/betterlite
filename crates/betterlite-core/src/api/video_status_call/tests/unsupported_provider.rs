use betterlite_error::error::ErrorClass;

use super::support::{
    Behavior, Bench, Mode, Outcome, PROVIDER, VIDEO_ID, call, failed, handles, lookups, request,
};
use crate::api::video_status_call::error::VideoStatusCallError;

#[test]
fn unsupported_provider_message_names_the_provider() {
    let mut bench = Bench::with(Behavior::Missing, Mode::Answers, Outcome::answers());
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(
        error.to_string(),
        "video status is not supported for gemini"
    );
    let VideoStatusCallError::Mapped {
        class: ErrorClass::ApiConnection(context),
        ..
    } = &error
    else {
        panic!("an unsupported provider is an api connection error");
    };
    assert_eq!(
        context.message,
        "GeminiException - video status is not supported for gemini"
    );
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some(PROVIDER));
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert!(handles(&bench.journal).is_empty());
    assert!(bench.fixture.state.optional_params.is_empty());
}

#[test]
fn unsupported_provider_selection_error_keeps_its_message() {
    let mut bench = Bench::with(Behavior::Unavailable, Mode::Answers, Outcome::answers());
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(
        error.to_string(),
        "no video config is installed for 'gemini'"
    );
    let VideoStatusCallError::Mapped {
        class: ErrorClass::ApiConnection(context),
        ..
    } = &error
    else {
        panic!("a selection failure is an api connection error");
    };
    assert_eq!(
        context.message,
        "GeminiException - no video config is installed for 'gemini'"
    );
    assert!(handles(&bench.journal).is_empty());
}

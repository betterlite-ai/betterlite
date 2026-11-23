use betterlite_error::error::ErrorClass;

use super::support::{
    Behavior, Bench, Outcome, PROVIDER, VIDEO_ID, call, failed, handles, lookups, request,
};
use crate::api::video_content_call::error::VideoContentCallError;

#[test]
fn unsupported_provider_message_names_the_provider() {
    let mut bench = Bench::with(Behavior::Missing, Outcome::Bytes(Vec::new()));
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(
        error.to_string(),
        "video support download is not supported for gemini"
    );
    let VideoContentCallError::Mapped { class, .. } = &error else {
        panic!("an unsupported provider is mapped, got {error:?}");
    };
    let ErrorClass::BadRequest(details) = class else {
        panic!("an unsupported provider answers 400, got {class:?}");
    };
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(
        details.message,
        "video support download is not supported for gemini"
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(PROVIDER));
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert!(handles(&bench.journal).is_empty());
    assert!(bench.fixture.state.optional_params.is_empty());
}

#[test]
fn unsupported_provider_selection_error_answers_400_with_its_message() {
    let mut bench = Bench::with(Behavior::Unavailable, Outcome::Bytes(Vec::new()));
    let error = failed(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(
        error.to_string(),
        "no video config is installed for 'gemini'"
    );
    let VideoContentCallError::Mapped { class, .. } = &error else {
        panic!("a selection failure is mapped, got {error:?}");
    };
    let ErrorClass::BadRequest(details) = class else {
        panic!("a selection failure answers 400, got {class:?}");
    };
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(details.message, "no video config is installed for 'gemini'");
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(PROVIDER));
    assert!(handles(&bench.journal).is_empty());
}

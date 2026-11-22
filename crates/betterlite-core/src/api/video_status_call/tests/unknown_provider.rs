use betterlite_error::error::ErrorClass;

use super::support::{Bench, Outcome, VIDEO_ID, call, failed, handles, lookups, request};
use crate::api::video_status_call::error::VideoStatusCallError;

#[test]
fn unknown_provider_name_fails_before_the_selector() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.custom_llm_provider = Some("videoprov".to_owned());
    let error = failed(call(&mut bench, request));
    assert_eq!(error.to_string(), "'videoprov' is not a valid LlmProviders");
    let VideoStatusCallError::Mapped { class, .. } = &error else {
        panic!("an unknown provider is mapped, got {error:?}");
    };
    let ErrorClass::BadRequest(details) = class else {
        panic!("an unknown provider answers 400, got {class:?}");
    };
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(details.message, "'videoprov' is not a valid LlmProviders");
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("videoprov"));
    assert!(lookups(&bench.journal).is_empty());
    assert!(handles(&bench.journal).is_empty());
}

#[test]
fn unknown_provider_empty_explicit_provider_is_kept_and_fails() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.custom_llm_provider = Some(String::new());
    let error = failed(call(&mut bench, request));
    assert_eq!(error.to_string(), "'' is not a valid LlmProviders");
    let VideoStatusCallError::Mapped { class, .. } = &error else {
        panic!("an empty provider is mapped, got {error:?}");
    };
    let ErrorClass::BadRequest(details) = class else {
        panic!("an empty provider answers 400, got {class:?}");
    };
    assert_eq!(class.metadata().status_code, 400);
    assert_eq!(details.message, "'' is not a valid LlmProviders");
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(""));
    assert!(lookups(&bench.journal).is_empty());
}

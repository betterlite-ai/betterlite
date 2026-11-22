use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{
    Bench, Outcome, PROVIDER, VIDEO_ID, call, failed, handled, handles, kwargs, lookups, ready,
    request, text,
};
use crate::api::video_content_call::error::VideoContentCallError;

#[test]
fn logging_update_payload_matches_litellm() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[
        ("litellm_call_id", text("call-9")),
        ("user", text("erik")),
        ("api_base", text("http://mock.test/v1")),
    ]);
    ready(call(&mut bench, request));
    let logged = handled(&bench.journal).logged;
    assert_eq!(logged.model, "");
    assert_eq!(logged.user.as_deref(), Some("erik"));
    assert_eq!(
        logged.optional_params,
        kwargs(&[("video_id", text(VIDEO_ID))])
    );
    assert_eq!(
        logged.litellm_params,
        kwargs(&[
            ("litellm_call_id", text("call-9")),
            ("video_id", text(VIDEO_ID)),
        ])
    );
    assert_eq!(logged.custom_llm_provider.as_deref(), Some(PROVIDER));
    assert_eq!(logged.provider_note, Some(text(PROVIDER)));
    assert_eq!(logged.call_type, "video_generation");
}

#[test]
fn logging_update_absent_values_become_null_and_none() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    ready(call(&mut bench, request(VIDEO_ID)));
    let logged = handled(&bench.journal).logged;
    assert_eq!(
        logged.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert!(logged.user.is_none());
    assert_eq!(logged.model, "");
}

#[test]
fn logging_update_wrong_user_type_is_a_bad_request_naming_the_field() {
    let mut bench = Bench::new(Outcome::Bytes(Vec::new()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("user", Json::Bool(true))]);
    let error = failed(call(&mut bench, request));
    assert_eq!(error.to_string(), "user must be a string");
    let VideoContentCallError::Mapped {
        class: ErrorClass::BadRequest(details),
        message,
    } = &error
    else {
        panic!("a wrong user type is a bad request");
    };
    assert_eq!(message, "user must be a string");
    assert_eq!(details.message, "user must be a string");
    assert_eq!(details.body_param.as_deref(), Some("user"));
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some(PROVIDER));
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert!(handles(&bench.journal).is_empty());
}

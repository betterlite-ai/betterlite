#![cfg(test)]

use betterlite_redact::secret::Redactor;

use super::cases::{STABILITY_MODEL, answered, auth_params, call, image_list, run};
use super::fixture::Reply;
use crate::bedrock_settings::BedrockError;
use crate::image_edits::error::ImageEditError;

pub(super) fn edit_http_error(redactor: &Redactor) {
    throttled_error(redactor);
    redirect_error(redactor);
    binary_error(redactor);
    timeout_error(redactor);
    transport_error(redactor);
}

fn provider_error(error: &ImageEditError) -> (u16, &BedrockError) {
    match error {
        ImageEditError::Provider { status, error } => (*status, error),
        other @ (ImageEditError::Canvas(_)
        | ImageEditError::CanvasReply(_)
        | ImageEditError::Stability(_)
        | ImageEditError::Signing(_)
        | ImageEditError::Endpoint(_)
        | ImageEditError::Header(_)
        | ImageEditError::Logging(_)
        | ImageEditError::Json(_)
        | ImageEditError::NullReply
        | ImageEditError::BodyShape
        | ImageEditError::Transport(_)) => panic!("expected a provider error, got {other:?}"),
    }
}

fn throttled_error(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut throttled_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    throttled_call.images = &supplied;
    let throttled = run(
        throttled_call,
        "{}",
        answered(Reply::status(
            429,
            &[("x-amzn-errortype", "ThrottlingException")],
            r#"{"message": "slow down"}"#,
        )),
        redactor,
    );
    let (status, error) = provider_error(throttled.error());
    assert_eq!(status, 429);
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.message, r#"{"message": "slow down"}"#);
    assert_eq!(
        error.response_headers,
        vec![(
            "x-amzn-errortype".to_owned(),
            "ThrottlingException".to_owned()
        )]
    );
    assert_eq!(throttled.error().status(), Some(429));
    assert!(
        throttled.post_call().is_none(),
        "a failed status logs no post_call"
    );
}

fn redirect_error(redactor: &Redactor) {
    // httpx treats every non-2xx as an error, so a redirect is a provider error too
    let supplied = image_list(&["aW1n"]);
    let mut redirect_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    redirect_call.images = &supplied;
    let redirect = run(
        redirect_call,
        "{}",
        answered(Reply::status(302, &[], r#"{"images": []}"#)),
        redactor,
    );
    assert_eq!(redirect.error().status(), Some(302));
    assert!(redirect.post_call().is_none());
}

fn binary_error(redactor: &Redactor) {
    // a body that is not UTF-8 still reports a provider error, naming its byte count
    let supplied = image_list(&["aW1n"]);
    let mut binary_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    binary_call.images = &supplied;
    let binary = run(
        binary_call,
        "{}",
        answered(Reply::bytes(500, &[], vec![0xFF, 0xFE])),
        redactor,
    );
    let (status, error) = provider_error(binary.error());
    assert_eq!(status, 500);
    assert_eq!(error.error.message, "2 bytes that are not UTF-8");
}

fn timeout_error(redactor: &Redactor) {
    // a timeout reports 408, and a failed transfer keeps the transport error
    let supplied = image_list(&["aW1n"]);
    let mut timed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    timed_call.images = &supplied;
    let timed = run(timed_call, "{}", answered(Reply::Timeout), redactor);
    let (status, error) = provider_error(timed.error());
    assert_eq!(status, 408);
    assert_eq!(error.error.message, "the provider request timed out");
    assert!(timed.post_call().is_none());
}

fn transport_error(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut broken_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    broken_call.images = &supplied;
    let broken = run(broken_call, "{}", answered(Reply::Failed), redactor);
    assert!(matches!(broken.error(), ImageEditError::Transport(_)));
    assert!(broken.post_call().is_none());
}

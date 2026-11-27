#![cfg(test)]

use betterlite_redact::secret::Redactor;

use super::cases::{Run, STABILITY_MODEL, answered, auth_params, call, image_list, run};
use super::fixture::Reply;
use crate::bedrock_settings::BedrockError;
use crate::image_edits::error::ImageEditError;

pub(super) fn edit_http_error(redactor: &Redactor) {
    throttled_error(redactor);
    binary_and_timeout_errors(redactor);
    redirect_and_transport_errors(redactor);
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

fn failed_call(redactor: &Redactor, reply: Reply) -> Run {
    let supplied = image_list(&["aW1n"]);
    let mut broken_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    broken_call.images = &supplied;
    run(broken_call, "{}", answered(reply), redactor)
}

fn throttled_error(redactor: &Redactor) {
    let throttled = failed_call(
        redactor,
        Reply::status(
            429,
            &[("x-amzn-errortype", "ThrottlingException")],
            r#"{"message": "slow down"}"#,
        ),
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

fn binary_and_timeout_errors(redactor: &Redactor) {
    // a body that is not UTF-8 still reports a provider error, naming its byte count
    let binary = failed_call(redactor, Reply::bytes(500, &[], vec![0xFF, 0xFE]));
    let (binary_status, binary_detail) = provider_error(binary.error());
    assert_eq!(binary_status, 500);
    assert_eq!(binary_detail.error.message, "2 bytes that are not UTF-8");
    // a timeout reports 408, and a failed transfer keeps the transport error
    let timed = failed_call(redactor, Reply::Timeout);
    let (timed_status, timed_detail) = provider_error(timed.error());
    assert_eq!(timed_status, 408);
    assert_eq!(timed_detail.error.message, "the provider request timed out");
    assert!(timed.post_call().is_none());
}

fn redirect_and_transport_errors(redactor: &Redactor) {
    // httpx treats every non-2xx as an error, so a redirect is a provider error too
    let redirect = failed_call(redactor, Reply::status(302, &[], r#"{"images": []}"#));
    assert_eq!(redirect.error().status(), Some(302));
    assert!(redirect.post_call().is_none());
    // a failed transfer hides as a transport error and logs no post_call
    let broken = failed_call(redactor, Reply::Failed);
    assert!(matches!(broken.error(), ImageEditError::Transport(_)));
    assert!(broken.post_call().is_none());
}

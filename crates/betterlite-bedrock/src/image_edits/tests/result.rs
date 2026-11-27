#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageUsage;
use betterlite_redact::secret::Redactor;

use super::cases::{
    CANVAS_COST, CANVAS_MODEL, STABILITY_COST, STABILITY_MODEL, answered, auth_params, call,
    image_list, run,
};
use super::fixture::{NOW_SECONDS, Reply, capture};
use crate::image_edits::error::ImageEditError;
use crate::stability_edits::error::{StabilityEditError, StabilityReplyError};

pub(super) fn edit_result_projection(redactor: &Redactor) {
    canvas_projection(redactor);
    stability_projection(redactor);
    null_reply(redactor);
    malformed_reply(redactor);
    list_reply(redactor);
    canvas_error_reply(redactor);
}

fn canvas_projection(redactor: &Redactor) {
    let (captured, _guard) = capture();
    let supplied = image_list(&["aW1n"]);
    // the canvas reply keeps the instant and a default usage record, and prices every returned image
    let mut canvas_call = call(CANVAS_MODEL, Some("edit it"), auth_params());
    canvas_call.images = &supplied;
    let canvas = run(
        canvas_call,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b25l", "dHdv"]}"#)),
        redactor,
    );
    let reply = canvas.response();
    assert_eq!(reply.response.created, NOW_SECONDS);
    assert_eq!(reply.response.usage, Some(ImageUsage::default()));
    assert_eq!(canvas.images(), ["b25l", "dHdv"]);
    assert_eq!(canvas.cost().as_deref(), Some("0.5"));
    assert_eq!(
        canvas.state.model_call_details.get("original_response"),
        Some(&Json::Text(r#"{"images": ["b25l", "dHdv"]}"#.to_owned()))
    );
    assert!(
        captured
            .text()
            .contains(r#"raw model_response: {"images": ["b25l", "dHdv"]}"#)
    );
}

fn stability_projection(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    // the stability reply has no usage record and prices the call once
    let mut stability_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    stability_call.images = &supplied;
    let stability = run(
        stability_call,
        STABILITY_COST,
        answered(Reply::ok(
            r#"{"images": ["c3RhYg=="], "finish_reasons": [null]}"#,
        )),
        redactor,
    );
    assert_eq!(stability.response().response.usage, None);
    assert_eq!(stability.images(), ["c3RhYg=="]);
    assert_eq!(stability.cost().as_deref(), Some("0.07"));
}

fn null_reply(redactor: &Redactor) {
    // a body of null stops at the handler's own check, after post_call already recorded the text
    let supplied = image_list(&["aW1n"]);
    let mut null_call = call(STABILITY_MODEL, None, auth_params());
    null_call.images = &supplied;
    let null = run(null_call, "{}", answered(Reply::ok("null")), redactor);
    assert!(matches!(null.error(), ImageEditError::NullReply));
    assert_eq!(
        null.error().to_string(),
        "Error in response object format, got None"
    );
    assert!(null.post_call().is_some());
}

fn malformed_reply(redactor: &Redactor) {
    // text that is not JSON is the outer parse failure, and post_call still ran first
    let supplied = image_list(&["aW1n"]);
    let mut malformed_call = call(STABILITY_MODEL, None, auth_params());
    malformed_call.images = &supplied;
    let malformed = run(
        malformed_call,
        "{}",
        answered(Reply::ok("{not json")),
        redactor,
    );
    assert!(matches!(malformed.error(), ImageEditError::Json(_)));
    assert!(malformed.post_call().is_some());
}

fn list_reply(redactor: &Redactor) {
    // a parsed list passes the outer check and the family names the shape
    let supplied = image_list(&["aW1n"]);
    let mut list_call = call(STABILITY_MODEL, None, auth_params());
    list_call.images = &supplied;
    let list = run(list_call, "{}", answered(Reply::ok("[]")), redactor);
    assert!(matches!(
        list.error(),
        ImageEditError::Stability(StabilityEditError::Reply(StabilityReplyError::NotAnObject))
    ));
    assert!(list.post_call().is_some());
}

fn canvas_error_reply(redactor: &Redactor) {
    // a canvas reply carrying errors reports through the canvas config at the upstream status
    let supplied = image_list(&["aW1n"]);
    let mut failed_call = call(CANVAS_MODEL, None, auth_params());
    failed_call.images = &supplied;
    let failed = run(
        failed_call,
        CANVAS_COST,
        answered(Reply::ok(r#"{"errors": ["content filtered"]}"#)),
        redactor,
    );
    assert!(matches!(failed.error(), ImageEditError::CanvasReply(_)));
    assert_eq!(failed.error().status(), Some(200));
}

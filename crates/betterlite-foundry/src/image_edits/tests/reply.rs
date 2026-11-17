use betterlite_openai::image_generation::reply::{
    ImageGenerationReply, ImageReplyError, ImageUsage,
};

use crate::image_edits::error::ImageEditError;
use crate::image_edits::reply::{decode_mai_edit_reply, transform_mai_edit_response};
use crate::mai_images::error::MaiImageError;

use super::support::{self, Logger, PostCallRecord};

const NOW: i64 = 1_700_000_000;

pub(super) fn mai_edit_usage() {
    normalize_log_and_project();
    clock_and_default_usage();
    size_is_not_synthesized();
    failures_keep_the_provider_status();
    malformed_bodies_fail();
}

fn normalize_log_and_project() {
    let body = br#"{"created": 7, "data": [{"b64_json": "aGk="}], "usage": {"num_input_text_tokens": 3, "num_input_image_tokens": 4, "output_image_tokens": 5}}"#;
    let mut logger = Logger::default();
    let reply = transform_mai_edit_response(body, 200, NOW, &mut logger).unwrap();
    normalized_reply(&reply);
    let call = logger.recorded.expect("one logged call");
    logged_call(&call);
}

fn normalized_reply(reply: &ImageGenerationReply) {
    assert_eq!(reply.created, 7);
    assert_eq!(reply.size, None);
    assert_eq!(reply.data.len(), 1);
    assert_eq!(reply.data[0].b64_json.as_deref(), Some("aGk="));
    assert_eq!(reply.usage.input_tokens, 7);
    assert_eq!(reply.usage.input_tokens_details.image_tokens, 4);
    assert_eq!(reply.usage.input_tokens_details.text_tokens, 3);
    assert_eq!(reply.usage.output_tokens, 5);
    assert_eq!(reply.usage.total_tokens, 12);
    assert_eq!(reply.usage.prompt_tokens, Some(7));
    assert_eq!(reply.usage.completion_tokens, Some(5));
}

fn logged_call(call: &PostCallRecord) {
    assert_eq!(call.input, "");
    assert_eq!(call.api_key.as_deref(), Some(""));
    assert!(call.complete_input_dict.is_empty());
    assert_eq!(
        call.original_response,
        support::object(concat!(
            r#"{"created": 7, "data": [{"b64_json": "aGk="}], "usage": {"#,
            r#""num_input_text_tokens": 3, "num_input_image_tokens": 4, "output_image_tokens": 5, "#,
            r#""input_tokens": 7, "output_tokens": 5, "total_tokens": 12, "#,
            r#""input_tokens_details": {"image_tokens": 4, "text_tokens": 3}}}"#
        ))
    );
}

fn clock_and_default_usage() {
    let empty_reply = decode_mai_edit_reply(br#"{"data": []}"#, 200, NOW).unwrap();
    assert_eq!(empty_reply.created, NOW);
    assert_eq!(empty_reply.usage, ImageUsage::default());
    assert!(empty_reply.data.is_empty());

    // an explicit null usage is normalized to zeros, so the usage is no longer absent
    let null_usage = decode_mai_edit_reply(br#"{"created": 1, "usage": null}"#, 200, NOW).unwrap();
    assert_eq!(null_usage.created, 1);
    assert_eq!(null_usage.usage.input_tokens, 0);
    assert_eq!(null_usage.usage.output_tokens, 0);
    assert_eq!(null_usage.usage.total_tokens, 0);
    assert_eq!(null_usage.usage.prompt_tokens, Some(0));
    assert_eq!(null_usage.usage.completion_tokens, Some(0));

    let counted = decode_mai_edit_reply(
        br#"{"usage": {"input_tokens": 2, "output_tokens": 3}}"#,
        200,
        NOW,
    )
    .unwrap();
    assert_eq!(counted.usage.input_tokens, 2);
    assert_eq!(counted.usage.output_tokens, 3);
    assert_eq!(counted.usage.total_tokens, 5);
    assert_eq!(counted.usage.prompt_tokens, Some(2));
    assert_eq!(counted.size, None);
}

fn size_is_not_synthesized() {
    let sized = decode_mai_edit_reply(br#"{"size": "1024x1024"}"#, 200, NOW).unwrap();
    assert_eq!(sized.size.as_deref(), Some("1024x1024"));

    let url_reply = decode_mai_edit_reply(
        br#"{"data": [{"url": "https://example.test/i.png"}]}"#,
        200,
        NOW,
    )
    .unwrap();
    assert_eq!(url_reply.size, None);
    assert_eq!(
        url_reply.data[0].url.as_deref(),
        Some("https://example.test/i.png")
    );
}

fn failures_keep_the_provider_status() {
    let error = decode_mai_edit_reply(b"<html>bad gateway</html>", 502, NOW).unwrap_err();
    let ImageEditError::Mai(MaiImageError::NotJson { record, .. }) = &error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(record.status_code, 502);
    assert_eq!(record.message, "<html>bad gateway</html>");
    assert_eq!(error.status_code(), Some(502));

    let not_object = decode_mai_edit_reply(b"[1]", 200, NOW).unwrap_err();
    assert!(matches!(
        not_object,
        ImageEditError::Mai(MaiImageError::NotAnObject)
    ));
    assert_eq!(not_object.status_code(), Some(500));
}

fn malformed_bodies_fail() {
    let field_type =
        decode_mai_edit_reply(br#"{"usage": {"input_tokens": true}}"#, 200, NOW).unwrap_err();
    assert!(matches!(
        field_type,
        ImageEditError::Mai(MaiImageError::FieldType {
            field: "input_tokens"
        })
    ));

    // a usage counter summed past i64::MAX fails with a 500, never a wrapped count
    let overflow = decode_mai_edit_reply(
        br#"{"usage": {"input_tokens": 9223372036854775807, "output_tokens": 1}}"#,
        200,
        NOW,
    )
    .unwrap_err();
    assert!(matches!(
        overflow,
        ImageEditError::Mai(MaiImageError::TokenOverflow { .. })
    ));
    assert_eq!(overflow.status_code(), Some(500));

    // a body with an error object is a failure through B004's projection, not an empty reply
    let projection =
        decode_mai_edit_reply(br#"{"error": {"message": "boom", "code": 500}}"#, 200, NOW)
            .unwrap_err();
    assert!(matches!(
        projection,
        ImageEditError::Projection(ImageReplyError::ResponseObject { status: 500, .. })
    ));
    assert_eq!(projection.status_code(), Some(500));
}

use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::reply::{
    ImageLogging, MaiImageReply, MaiImageResponse, transform_image_generation_response,
};

use super::support::{RecordedLogging, object};

fn transform(
    body: &str,
    optional_params: &str,
    request_data: &str,
    logging: &mut dyn ImageLogging,
) -> Result<MaiImageResponse, MaiImageError> {
    let key = SecretString::from("sk-fixture");
    transform_image_generation_response(
        &MaiImageReply {
            body: body.as_bytes(),
            status_code: 200,
            request_data: &object(request_data),
            optional_params: &object(optional_params),
            api_key: Some(&key),
        },
        logging,
    )
}

const ZERO_USAGE: &str = r#"{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"total_tokens":0}"#;

pub(super) fn response_size() {
    let mut logging = RecordedLogging::default();
    let reply = transform(
        r#"{"data":[{"b64_json":"AAAA","size":"1x1"}]}"#,
        "{}",
        r#"{"prompt":"a red fox"}"#,
        &mut logging,
    )
    .unwrap();
    // an absent usage stays absent and the size ignores what the images say
    assert_eq!(reply.size, "1024x1024");
    assert_eq!(
        reply.body,
        object(r#"{"data":[{"b64_json":"AAAA","size":"1x1"}]}"#)
    );
    assert_eq!(logging.calls.len(), 1);
    let call = logging.calls.first().unwrap();
    assert_eq!(call.input, "a red fox");
    assert_eq!(call.api_key.as_deref(), Some("sk-fixture"));
    assert_eq!(
        call.complete_input_dict,
        object(r#"{"prompt":"a red fox"}"#)
    );
    assert_eq!(call.original_response, reply.body);
    let sized = transform(
        r#"{"data":[]}"#,
        r#"{"width":800,"height":900}"#,
        "{}",
        &mut logging,
    )
    .unwrap();
    assert_eq!(sized.size, "800x900");
    assert_eq!(logging.calls.last().unwrap().input, "");
    let partly = transform(
        "{}",
        r#"{"height":640}"#,
        r#"{"prompt":null}"#,
        &mut logging,
    )
    .unwrap();
    assert_eq!(partly.size, "1024x640");
    let text = transform("{}", r#"{"width":"512","height":null}"#, "{}", &mut logging).unwrap();
    assert_eq!(text.size, "512x1024");
    let with_usage = transform(
        r#"{"data":[],"usage":{"num_input_text_tokens":3,"num_output_tokens":4}}"#,
        "{}",
        r#"{"prompt":"p"}"#,
        &mut logging,
    )
    .unwrap();
    let expected = object(
        r#"{"num_input_text_tokens":3,"num_output_tokens":4,"input_tokens":3,"input_tokens_details":{"image_tokens":0,"text_tokens":3},"output_tokens":4,"total_tokens":7}"#,
    );
    let logged = logging.calls.last().unwrap();
    assert_eq!(
        with_usage.body.get("usage").and_then(Json::as_object),
        Some(&expected)
    );
    assert_eq!(logged.original_response, with_usage.body);
    let null_usage = transform(r#"{"usage":null}"#, "{}", "{}", &mut logging).unwrap();
    assert_eq!(
        null_usage.body.get("usage").and_then(Json::as_object),
        Some(&object(ZERO_USAGE))
    );
    reply_failures();
}

fn reply_failures() {
    let mut logging = RecordedLogging::default();
    let key = SecretString::from("sk-fixture");
    let request_data = object(r#"{"prompt":"p"}"#);
    let optional_params = object("{}");
    let reply = MaiImageReply {
        body: b"<html>bad gateway</html>",
        status_code: 502,
        request_data: &request_data,
        optional_params: &optional_params,
        api_key: Some(&key),
    };
    let error = transform_image_generation_response(&reply, &mut logging).unwrap_err();
    assert!(matches!(
        &error,
        MaiImageError::NotJson { record, .. }
            if record.status_code == 502 && record.message == "<html>bad gateway</html>"
    ));
    assert_eq!(error.to_string(), "<html>bad gateway</html>");
    assert_eq!(error.status_code(), Some(502));
    let invalid_utf8 = MaiImageReply {
        body: b"\xffnot json",
        ..reply
    };
    let lossy = transform_image_generation_response(&invalid_utf8, &mut logging).unwrap_err();
    assert_eq!(lossy.to_string(), "\u{FFFD}not json");
    assert!(logging.calls.is_empty());
    let not_object = transform("[]", "{}", "{}", &mut logging).unwrap_err();
    assert!(matches!(not_object, MaiImageError::NotAnObject));
    assert_eq!(not_object.status_code(), Some(500));
    let bad_usage = transform(r#"{"usage":"many"}"#, "{}", "{}", &mut logging).unwrap_err();
    assert!(matches!(
        bad_usage,
        MaiImageError::FieldType { field: "usage" }
    ));
    let bad_prompt = transform("{}", "{}", r#"{"prompt":4}"#, &mut logging).unwrap_err();
    assert!(matches!(
        bad_prompt,
        MaiImageError::FieldType { field: "prompt" }
    ));
    let bad_width = transform("{}", r#"{"width":[1]}"#, "{}", &mut logging).unwrap_err();
    assert!(matches!(
        bad_width,
        MaiImageError::FieldType { field: "width" }
    ));
}

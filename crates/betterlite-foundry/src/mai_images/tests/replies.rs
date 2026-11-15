use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::mai_images::error::MaiImageError;
use crate::mai_images::reply::{
    ImageLogging, MaiImageReply, MaiImageResponse, transform_image_generation_response,
};

use super::support::{RecordedLogging, kind, object, variant};

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

fn usage_of(response: &MaiImageResponse) -> Option<&Object> {
    response.body.get("usage").and_then(Json::as_object)
}

pub(super) fn response_size() {
    absent_usage_and_logging();
    sizes_come_from_the_request();
    usage_is_normalized();
    reply_failures();
}

fn absent_usage_and_logging() {
    let mut logging = RecordedLogging::default();
    let body = r#"{"data":[{"b64_json":"AAAA","size":"1x1"}]}"#;
    let reply = transform(body, "{}", r#"{"prompt":"a red fox"}"#, &mut logging).unwrap();
    // an absent usage stays absent and the size ignores what the images say
    assert_eq!(reply.size, "1024x1024");
    assert_eq!(reply.body, object(body));
    assert_eq!(logging.calls.len(), 1);
    let call = logging.calls.first().unwrap();
    assert_eq!(call.input, "a red fox");
    assert_eq!(call.api_key.as_deref(), Some("sk-fixture"));
    assert_eq!(
        call.complete_input_dict,
        object(r#"{"prompt":"a red fox"}"#)
    );
    assert_eq!(call.original_response, reply.body);
    transform("{}", "{}", "{}", &mut logging).unwrap();
    assert_eq!(logging.calls.last().unwrap().input, "");
    transform("{}", "{}", r#"{"prompt":null}"#, &mut logging).unwrap();
    assert_eq!(logging.calls.last().unwrap().input, "");
}

fn sizes_come_from_the_request() {
    let mut logging = RecordedLogging::default();
    let mut sized = |optional: &str| transform("{}", optional, "{}", &mut logging).unwrap().size;
    assert_eq!(sized(r#"{"width":800,"height":900}"#), "800x900");
    assert_eq!(sized(r#"{"height":640}"#), "1024x640");
    assert_eq!(sized(r#"{"width":"512","height":null}"#), "512x1024");
}

fn usage_is_normalized() {
    let mut logging = RecordedLogging::default();
    let body = r#"{"data":[],"usage":{"num_input_text_tokens":3,"num_output_tokens":4}}"#;
    let reply = transform(body, "{}", r#"{"prompt":"p"}"#, &mut logging).unwrap();
    let expected = object(
        r#"{"num_input_text_tokens":3,"num_output_tokens":4,"input_tokens":3,"input_tokens_details":{"image_tokens":0,"text_tokens":3},"output_tokens":4,"total_tokens":7}"#,
    );
    assert_eq!(usage_of(&reply), Some(&expected));
    assert_eq!(logging.calls.last().unwrap().original_response, reply.body);
    let zeros = object(
        r#"{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"total_tokens":0}"#,
    );
    let null_usage = transform(r#"{"usage":null}"#, "{}", "{}", &mut logging).unwrap();
    assert_eq!(usage_of(&null_usage), Some(&zeros));
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
    assert_eq!(error.llm_provider(), None);
    let invalid_utf8 = MaiImageReply {
        body: b"\xffnot json",
        ..reply
    };
    let lossy = transform_image_generation_response(&invalid_utf8, &mut logging).unwrap_err();
    assert_eq!(lossy.to_string(), "\u{FFFD}not json");
    assert!(logging.calls.is_empty());
    unusable_replies();
}

fn unusable_replies() {
    let mut logging = RecordedLogging::default();
    let mut failure = |body: &str, optional: &str, request: &str| {
        variant(&transform(body, optional, request, &mut logging).unwrap_err())
    };
    assert_eq!(failure("[]", "{}", "{}"), "not_an_object");
    assert_eq!(failure(r#"{"usage":"many"}"#, "{}", "{}"), "field_type");
    assert_eq!(failure("{}", "{}", r#"{"prompt":4}"#), "field_type");
    assert_eq!(failure("{}", r#"{"width":[1]}"#, "{}"), "field_type");
    assert_eq!(kind(transform("{}", "{}", "{}", &mut logging)), "ok");
}

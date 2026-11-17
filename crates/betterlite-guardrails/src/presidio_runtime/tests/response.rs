use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    content_of, guard, guard_args, mask_with_mock, messages_of, quiet, request, response_body,
    response_content, tokens,
};

const CALL_REQUEST: &str =
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"call me at 555-0100"}]}"#;
const CALL_RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"call <PHONE_NUMBER_1>"},"finish_reason":"stop"}]}"#;

pub(super) async fn post_call_unmasks_the_response() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let guard = guard(&args);
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let mut body = response_body(CALL_RESPONSE);
    guard
        .post_call_success(&mut data, &mut body)
        .await
        .expect("the response is unmasked");
    assert_eq!(response_content(&body).as_deref(), Some("call 555-0100"));
}

pub(super) async fn logging_hook_masks_the_request() {
    let server = quiet();
    let mut args = guard_args(&server);
    mask_with_mock(&mut args);
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    let mut body = response_body(CALL_RESPONSE);
    guard
        .logging_hook(&mut data, &mut body, "completion")
        .expect("the request is masked for logging");
    assert_eq!(
        content_of(&messages_of(&data)[0]),
        GuardrailValue::Text("MASKED".to_owned())
    );
}

pub(super) async fn async_logging_hook_masks_the_request() {
    let server = quiet();
    let mut args = guard_args(&server);
    mask_with_mock(&mut args);
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    let mut body = response_body(CALL_RESPONSE);
    guard
        .async_logging_hook(&mut data, &mut body, "completion")
        .await
        .expect("the request is masked for logging");
    assert_eq!(
        content_of(&messages_of(&data)[0]),
        GuardrailValue::Text("MASKED".to_owned())
    );
}

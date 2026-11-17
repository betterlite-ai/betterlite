use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::support::{
    Mock, content_of, guard, guard_args, mask_with_mock, messages_of, quiet, request,
    response_body, response_content, seen, tokens,
};
use crate::guardrail_initializers::presidio::PresidioArgs;
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_runtime::PresidioGuard;

type Data = BTreeMap<String, GuardrailValue>;

const CALL_REQUEST: &str =
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"call me at 555-0100"}]}"#;
const CALL_RESPONSE: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"call <PHONE_NUMBER_1>"},"finish_reason":"stop"}]}"#;

fn logging_args(server: &Mock) -> PresidioArgs {
    let mut args = guard_args(server);
    mask_with_mock(&mut args);
    args
}

fn logging_guard(server: &Mock) -> PresidioGuard {
    guard(&logging_args(server))
}

fn payload() -> (Data, ResponseBody) {
    (request(CALL_REQUEST), response_body(CALL_RESPONSE))
}

fn masked_content(data: &Data) -> GuardrailValue {
    content_of(&messages_of(data)[0])
}

fn masked_for_logging(guard: &PresidioGuard) -> Data {
    let (mut data, mut body) = payload();
    guard
        .logging_hook(&mut data, &mut body, "completion")
        .expect("the request is masked for logging");
    data
}

async fn masked_for_async_logging(guard: &PresidioGuard) -> Data {
    let (mut data, mut body) = payload();
    guard
        .async_logging_hook(&mut data, &mut body, "completion")
        .await
        .expect("the request is masked for logging");
    data
}

pub(super) async fn post_call_unmasks_the_response() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let mut data = tokens(&[("<PHONE_NUMBER_1>", "555-0100")]);
    let mut body = response_body(CALL_RESPONSE);
    guard(&args)
        .post_call_success(&mut data, &mut body)
        .await
        .expect("the response is unmasked");
    assert_eq!(response_content(&body).as_deref(), Some("call 555-0100"));
}

fn assert_masked(data: &Data) {
    assert_eq!(
        masked_content(data),
        GuardrailValue::Text("MASKED".to_owned())
    );
}

pub(super) fn logging_hook_masks_the_request() {
    let server = quiet();
    let data = masked_for_logging(&logging_guard(&server));
    assert_masked(&data);
}

pub(super) async fn async_logging_hook_masks_the_request() {
    let server = quiet();
    let data = masked_for_async_logging(&logging_guard(&server)).await;
    assert_masked(&data);
    assert!(seen(&server).is_empty());
}

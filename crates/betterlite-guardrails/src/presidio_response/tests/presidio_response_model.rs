#![cfg(test)]

use super::shared::{
    analyzer, anonymizer, capture_logs, content_text, context, logged_count, logged_text,
    request_data,
};
use crate::presidio_response::Mode;
use crate::presidio_response::model::process_response_for_pii;
use crate::presidio_response::wire::{
    ChatModelResponse, ChatResponseMessage, Content, ContentItem, ToolCallItem,
};

fn response(json: &str) -> ChatModelResponse {
    serde_json::from_str(json).expect("model response")
}

fn first_message(response: &ChatModelResponse) -> &ChatResponseMessage {
    response
        .choices
        .as_ref()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.message.as_ref())
        .expect("first message")
}

#[tokio::test]
async fn masks_string_content() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        response(r#"{"choices":[{"message":{"role":"assistant","content":"Alice"}}]}"#);

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    assert_eq!(
        content_text(first_message(&response).content.as_ref()).as_deref(),
        Some("MASKED")
    );
    assert_eq!(logged_count(&data), 1);
}

#[tokio::test]
async fn masks_content_parts_and_skips_unsupported_items() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response(
        r#"{"choices":[{"message":{"role":"assistant","content":[{"type":"text","text":"Alice"},"bare",{"type":"text"}]}}]}"#,
    );

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    let Some(Content::Blocks(items)) = first_message(&response).content.as_ref() else {
        panic!("the content stays a list");
    };
    assert_eq!(items.len(), 3);
    let ContentItem::Object(first) = &items[0] else {
        panic!("the first part is an object");
    };
    assert_eq!(first.text.as_deref(), Some("MASKED"));
    assert!(matches!(items[1], ContentItem::Other(_)));
    let ContentItem::Object(empty) = &items[2] else {
        panic!("the textless part is an object");
    };
    assert!(empty.text.is_none());
}

#[tokio::test]
async fn masks_tool_calls_and_legacy_function_calls() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response(
        r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"Alice"}}],"function_call":{"name":"legacy","arguments":"Bobby"}}}]}"#,
    );

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    let message = first_message(&response);
    let Some(calls) = message.tool_calls.as_ref() else {
        panic!("tool calls survive");
    };
    let ToolCallItem::Object(call) = &calls[0] else {
        panic!("the tool call is an object");
    };
    let function = call.function.as_ref().expect("function");
    assert_eq!(function.arguments.as_deref(), Some("MASKED"));
    let function_call = message.function_call.as_ref().expect("function call");
    assert_eq!(function_call.arguments.as_deref(), Some("MASKED"));
    assert_eq!(logged_count(&data), 2);
}

#[tokio::test]
async fn unmasks_content_and_arguments() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data =
        request_data(r#"{"metadata":{"pii_tokens":{"<PERSON_1>":"Alice","<EMAIL_2>":"a@b.c"}}}"#);
    let mut response = response(
        r#"{"choices":[{"message":{"content":"Hi <PERSON_1>","tool_calls":[{"function":{"arguments":"<EMAIL_2>"}}],"function_call":{"arguments":"<PERSON_1>"}}}]}"#,
    );

    process_response_for_pii(context, &mut response, &mut data, Mode::Unmask)
        .await
        .expect("response processing");

    assert_eq!(
        content_text(first_message(&response).content.as_ref()).as_deref(),
        Some("Hi Alice")
    );
    let message = first_message(&response);
    let Some(calls) = message.tool_calls.as_ref() else {
        panic!("tool calls survive");
    };
    let ToolCallItem::Object(call) = &calls[0] else {
        panic!("the tool call is an object");
    };
    assert_eq!(
        call.function
            .as_ref()
            .and_then(|function| function.arguments.as_deref()),
        Some("a@b.c")
    );
    assert_eq!(
        message
            .function_call
            .as_ref()
            .and_then(|function| function.arguments.as_deref()),
        Some("Alice")
    );
}

#[tokio::test]
async fn skips_missing_and_unsupported_fields() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response(
        r#"{"choices":[{"message":null},{"delta":{"content":"streamed"}},{"message":{"content":{"unexpected":true},"tool_calls":[{"function":{"arguments":5}}]}}]}"#,
    );

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    let choices = response.choices.as_ref().expect("choices");
    assert_eq!(choices.len(), 3);
    let Some(Content::Other(raw)) = choices[2]
        .message
        .as_ref()
        .and_then(|message| message.content.as_ref())
    else {
        panic!("the unexpected content stays raw");
    };
    assert_eq!(raw.get(), r#"{"unexpected":true}"#);
    let arguments = choices[2]
        .message
        .as_ref()
        .and_then(|message| message.tool_calls.as_ref())
        .and_then(|calls| calls.first())
        .and_then(|item| match item {
            ToolCallItem::Object(call) => call.function.as_ref(),
            ToolCallItem::Other(_) => None,
        })
        .and_then(|function| function.extra.get("arguments"));
    assert_eq!(arguments.map(|argument| argument.get()), Some("5"));
    assert_eq!(logged_count(&data), 0);
}

#[tokio::test]
async fn keeps_unknown_fields() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response(
        r#"{"id":"chat-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","name":"helper","content":"Alice"}}]}"#,
    );

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    let raw = serde_json::to_string(&response).expect("response JSON");
    assert_eq!(
        raw,
        r#"{"choices":[{"index":0,"message":{"content":"MASKED","name":"helper","role":"assistant"}}],"id":"chat-1","object":"chat.completion"}"#
    );
}

#[tokio::test]
async fn unmasking_without_tokens_logs_a_debug_line() {
    let (captured, _guard) = capture_logs();
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        response(r#"{"choices":[{"message":{"role":"assistant","content":"Hi <PERSON_1>"}}]}"#);

    process_response_for_pii(context, &mut response, &mut data, Mode::Unmask)
        .await
        .expect("response processing");

    assert!(logged_text(&captured).contains("pii_tokens"));
}

#[tokio::test]
async fn an_empty_choices_list_is_left_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response(r#"{"choices":[]}"#);

    process_response_for_pii(context, &mut response, &mut data, Mode::Mask)
        .await
        .expect("response processing");

    assert_eq!(logged_count(&data), 0);
}

#![cfg(test)]

use super::shared::{
    analyzer, anonymizer, anthropic_texts, chat_content, context, request_data, response_body,
};
use crate::presidio_response::post_call::async_post_call_success_hook;

const TOKENS: &str = r#"{"metadata":{"pii_tokens":{"<PERSON_1>":"Alice"}}}"#;
const CHAT_WITH_TOKEN: &str =
    r#"{"choices":[{"message":{"role":"assistant","content":"Hi <PERSON_1>"}}]}"#;
const ANTHROPIC_WITH_TOKEN: &str =
    r#"{"type":"message","content":[{"type":"text","text":"Hi <PERSON_1>"}]}"#;

#[tokio::test]
async fn apply_to_output_masks_the_chat_response() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.apply_to_output = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response_body(CHAT_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
}

#[tokio::test]
async fn apply_to_output_masks_the_anthropic_response() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.apply_to_output = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response = response_body(ANTHROPIC_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(anthropic_texts(&response), vec!["MASKED".to_owned()]);
}

#[tokio::test]
async fn apply_to_output_does_not_unmask() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.apply_to_output = true;
    analyzer.settings.output_parse_pii = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data(TOKENS);
    let mut response = response_body(CHAT_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
}

#[tokio::test]
async fn output_parse_pii_unmasks_the_chat_response() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.output_parse_pii = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data(TOKENS);
    let mut response = response_body(CHAT_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(chat_content(&response).as_deref(), Some("Hi Alice"));
}

#[tokio::test]
async fn output_parse_pii_unmasks_the_anthropic_response() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.output_parse_pii = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data(TOKENS);
    let mut response = response_body(ANTHROPIC_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(anthropic_texts(&response), vec!["Hi Alice".to_owned()]);
}

#[tokio::test]
async fn the_global_output_parse_pii_setting_unmasks() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let mut context = context(&analyzer, &anonymizer);
    context.global_output_parse_pii = true;
    let mut data = request_data(TOKENS);
    let mut response = response_body(CHAT_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(chat_content(&response).as_deref(), Some("Hi Alice"));
}

#[tokio::test]
async fn neither_setting_leaves_the_response_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data(TOKENS);
    let mut response = response_body(CHAT_WITH_TOKEN);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(chat_content(&response).as_deref(), Some("Hi <PERSON_1>"));
}

#[tokio::test]
async fn an_embedding_response_stays_untouched_with_apply_to_output() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.apply_to_output = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let body = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2]}],"model":"embed"}"#;
    let mut response = response_body(body);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(response.into_raw().expect("response JSON").get(), body);
}

#[tokio::test]
async fn a_stream_chunk_stays_untouched_with_apply_to_output() {
    let mut analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    analyzer.settings.apply_to_output = true;
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let body = r#"{"object":"chat.completion.chunk","choices":[{"delta":{"content":"Hi"}}]}"#;
    let mut response = response_body(body);

    async_post_call_success_hook(context, &mut data, &mut response)
        .await
        .expect("post call hook");

    assert_eq!(response.into_raw().expect("response JSON").get(), body);
}

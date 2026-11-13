#![cfg(test)]

use super::shared::{
    analyzer, anonymizer, anthropic_texts, chat_content, context, request_data, response_body,
};
use crate::presidio_response::post_call::mask_output_response;

#[tokio::test]
async fn masks_a_chat_model_response() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        response_body(r#"{"choices":[{"message":{"role":"assistant","content":"Alice"}}]}"#);

    mask_output_response(context, &mut response, &mut data)
        .await
        .expect("output masking");

    assert_eq!(chat_content(&response).as_deref(), Some("MASKED"));
}

#[tokio::test]
async fn leaves_a_stream_chunk_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let body = r#"{"object":"chat.completion.chunk","choices":[{"delta":{"content":"Alice"}}]}"#;
    let mut response = response_body(body);

    mask_output_response(context, &mut response, &mut data)
        .await
        .expect("output masking");

    assert_eq!(response.into_raw().expect("response JSON").get(), body);
}

#[tokio::test]
async fn leaves_an_embedding_response_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let body =
        r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1]}],"model":"embed"}"#;
    let mut response = response_body(body);

    mask_output_response(context, &mut response, &mut data)
        .await
        .expect("output masking");

    assert_eq!(response.into_raw().expect("response JSON").get(), body);
}

#[tokio::test]
async fn leaves_an_anthropic_message_untouched() {
    let analyzer = analyzer("http://presidio.invalid", Some("MASKED"));
    let anonymizer = anonymizer("http://presidio.invalid");
    let context = context(&analyzer, &anonymizer);
    let mut data = request_data("{}");
    let mut response =
        response_body(r#"{"type":"message","content":[{"type":"text","text":"<PERSON_1>"}]}"#);

    mask_output_response(context, &mut response, &mut data)
        .await
        .expect("output masking");

    assert_eq!(anthropic_texts(&response), vec!["<PERSON_1>".to_owned()]);
}

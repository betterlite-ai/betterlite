#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::shadow_evidence::chat_text;
use crate::shadow_requests::Object;
use crate::shadow_requests::error::ShadowRequestError;
use crate::shadow_requests::surfaces::{
    ChatRequestFn, ChatRequestFuture, Surface, SurfaceOps, surface_ops,
};

use super::judgeable_sample;

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn chat_reply(text: &str) -> Json {
    let source =
        format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{text}"}}}}]}}"#);
    serde_json::from_str(&source).unwrap()
}

fn responses_reply(text: &str) -> Json {
    let source = format!(
        r#"{{"id":"resp_1","created_at":1,"output":[{{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{{"type":"output_text","text":"{text}","annotations":[]}}]}}]}}"#
    );
    serde_json::from_str(&source).unwrap()
}

fn searching_request<'request>(
    _kwargs: &'request Object,
    _model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async {
        Ok(object(
            r#"{"messages":[{"role":"user","content":"hi"}],"web_search_options":{}}"#,
        ))
    })
}

fn failing_request<'request>(
    _kwargs: &'request Object,
    _model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async { Err(ShadowRequestError::ModelNotText) })
}

fn string_messages_request<'request>(
    _kwargs: &'request Object,
    _model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async { Ok(object(r#"{"messages":["hi"]}"#)) })
}

fn custom_ops(chat_request: ChatRequestFn) -> SurfaceOps {
    SurfaceOps {
        chat_request,
        final_text: chat_text::final_text,
        wire_params: false,
        surface: Surface::Chat,
    }
}

#[tokio::test]
async fn hosted_web_search_in_the_wire_body_skips_the_sample() {
    let kwargs = object(
        r#"{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":"hi"}],
            "litellm_params":{"proxy_server_request":{"body":{"tools":[{"type":"web_search"}]}}}}"#,
    );
    let sample = judgeable_sample(
        surface_ops("anthropic_messages").unwrap(),
        &kwargs,
        &object("{}"),
        &chat_reply("answer"),
    )
    .await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn hosted_web_search_in_model_parameters_skips_the_chat_sample() {
    let kwargs = object(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    let model_parameters = object(r#"{"web_search_options":{}}"#);
    let sample = judgeable_sample(
        surface_ops("completion").unwrap(),
        &kwargs,
        &model_parameters,
        &chat_reply("answer"),
    )
    .await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn the_chat_surface_reads_model_parameters_not_the_wire_body() {
    let kwargs = object(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],
            "litellm_params":{"proxy_server_request":{"body":{"tools":[{"type":"web_search"}]}}}}"#,
    );
    let sample = judgeable_sample(
        surface_ops("acompletion").unwrap(),
        &kwargs,
        &object("{}"),
        &chat_reply("answer"),
    )
    .await
    .unwrap();
    assert_eq!(sample.0, vec![object(r#"{"role":"user","content":"hi"}"#)]);
    assert_eq!(sample.2, "answer");
}

#[tokio::test]
async fn hosted_web_search_added_by_the_translation_skips_the_sample() {
    let ops = custom_ops(searching_request);
    let sample = judgeable_sample(&ops, &object("{}"), &object("{}"), &chat_reply("answer")).await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn a_failing_request_build_skips_the_sample() {
    let ops = custom_ops(failing_request);
    let sample = judgeable_sample(&ops, &object("{}"), &object("{}"), &chat_reply("answer")).await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn messages_that_are_not_an_array_of_objects_skip_the_sample() {
    let ops = custom_ops(string_messages_request);
    let sample = judgeable_sample(&ops, &object("{}"), &object("{}"), &chat_reply("answer")).await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn an_empty_conversation_skips_the_sample() {
    let sample = judgeable_sample(
        surface_ops("completion").unwrap(),
        &object("{}"),
        &object("{}"),
        &chat_reply("answer"),
    )
    .await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn an_empty_final_text_skips_the_sample() {
    let kwargs = object(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    let sample = judgeable_sample(
        surface_ops("completion").unwrap(),
        &kwargs,
        &object("{}"),
        &chat_reply(""),
    )
    .await;
    assert!(sample.is_none());
}

#[tokio::test]
async fn a_usable_sample_returns_messages_params_and_real_text() {
    let kwargs = object(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    let model_parameters =
        object(r#"{"temperature":0.5,"stop":[],"top_p":null,"user":"u","tools":[]}"#);
    let sample = judgeable_sample(
        surface_ops("completion").unwrap(),
        &kwargs,
        &model_parameters,
        &chat_reply("answer"),
    )
    .await
    .unwrap();
    assert_eq!(sample.0, vec![object(r#"{"role":"user","content":"hi"}"#)]);
    assert_eq!(sample.1, object(r#"{"temperature":0.5,"user":"u"}"#));
    assert_eq!(sample.2, "answer");
}

#[tokio::test]
async fn empty_objects_and_strings_stay_in_the_forwarded_params() {
    let kwargs = object(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    let model_parameters = object(r#"{"response_format":{},"user":""}"#);
    let sample = judgeable_sample(
        surface_ops("completion").unwrap(),
        &kwargs,
        &model_parameters,
        &chat_reply("answer"),
    )
    .await
    .unwrap();
    assert_eq!(sample.1, object(r#"{"response_format":{},"user":""}"#));
}

#[tokio::test]
async fn the_responses_surface_samples_with_its_own_text_reader() {
    let kwargs = object(r#"{"model":"gpt-4o","messages":"hello"}"#);
    let sample = judgeable_sample(
        surface_ops("responses").unwrap(),
        &kwargs,
        &object("{}"),
        &responses_reply("final"),
    )
    .await
    .unwrap();
    assert_eq!(
        sample.0,
        vec![object(r#"{"role":"user","content":"hello"}"#)]
    );
    assert_eq!(sample.1, object("{}"));
    assert_eq!(sample.2, "final");
}

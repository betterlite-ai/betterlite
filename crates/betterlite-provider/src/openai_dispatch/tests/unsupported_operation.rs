use super::support;
use crate::aiohttp_openai_dispatch::AiohttpOpenAiRequest;
use crate::openai_dispatch::call::{ChatCall, OpenAiOperation};
use crate::openai_dispatch::dispatch::dispatch;
use crate::openai_dispatch::error::OpenAiProviderError;
use crate::openai_dispatch::fixtures::{
    FixtureAnswer, FixtureLogger, FixtureTextSource, TextReply,
};
use crate::openai_dispatch::transport::Deps;
use crate::operation::OperationRequest;

// The closed provider entry runs only an OpenAI request; any other request is refused before anything is sent.
#[tokio::test]
async fn openai_provider_unsupported_operation() {
    let composition = support::composition();
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let text = FixtureTextSource::new(TextReply::new(200, Vec::new()));
    let mut logger = FixtureLogger::default();
    let call = ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        ),
        credentials: support::credentials(None, None, None),
        headers: None,
        extra_headers: None,
        timeout: None,
        metadata: None,
    };
    let error = dispatch(
        OperationRequest::AiohttpOpenAi(AiohttpOpenAiRequest),
        OpenAiOperation::Chat(Box::new(call)),
        &composition,
        Deps {
            answer: &answer,
            text: &text,
            now: support::fixed_now,
            next_id: support::fixed_id,
        },
        &mut logger,
    )
    .await
    .expect_err("the unsupported request");
    assert!(matches!(error, OpenAiProviderError::UnsupportedOperation));
    assert_eq!(
        error.to_string(),
        "the openai provider composition runs only chat, embeddings and text-completion operations"
    );
    assert!(answer.requests().is_empty());
    assert!(logger.calls.is_empty());
}

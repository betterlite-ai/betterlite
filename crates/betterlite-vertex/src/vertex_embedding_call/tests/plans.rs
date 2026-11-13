#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_openai::embedding::EmbeddingInput;

use crate::vertex_embedding_call::error::VertexEmbeddingCallError;
use crate::vertex_embedding_call::multimodal::{
    DEFAULT_TIMEOUT, MULTIMODAL_EMBEDDING_MODELS, MultimodalClients, multimodal_embedding,
    multimodal_timeout,
};
use crate::vertex_embedding_call::post::{ClientTimeout, HANDLER_DEFAULT_TIMEOUT, Timeout};
use crate::vertex_embedding_call::text::{TextClients, text_client_timeout, text_embedding};

use super::support::{
    MULTIMODAL_ANSWER, MULTIMODAL_MODEL, NO_HEADERS, PostRecorder, TEXT_ANSWER, TOKEN, auth,
    catalog, extra, json, multimodal_call, reply, text_auth, text_call,
};

#[tokio::test]
async fn plans_text_psc() {
    let optional = json(r#"{"use_psc_endpoint_format": true}"#);
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.api_base = Some("https://private.example.com/");
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://private.example.com/v1/projects/project-p/locations/us-central1/endpoints/text-embedding-005:predict"
    );
}

#[tokio::test]
async fn plans_text_psc_flag_must_be_a_bool() {
    let optional = json(r#"{"use_psc_endpoint_format": "yes"}"#);
    let input = EmbeddingInput::Text(String::from("hello"));
    let call = text_call(&input, &optional);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let error = text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexEmbeddingCallError::Flag {
            field: "use_psc_endpoint_format"
        }
    ));
    assert_eq!(error.status_code(), 400);
    assert!(recorder.recorded().is_empty());
}

#[tokio::test]
async fn plans_text_zero_timeout() {
    assert_eq!(
        text_client_timeout(Some(Duration::ZERO)),
        ClientTimeout::HandlerDefault
    );
    assert_eq!(text_client_timeout(None), ClientTimeout::HandlerDefault);
    let thirty = ClientTimeout::Explicit(Timeout {
        total: Duration::from_secs(30),
        connect: Duration::from_secs(30),
    });
    assert_eq!(text_client_timeout(Some(Duration::from_secs(30))), thirty);

    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let recorder = Arc::new(PostRecorder::scripted(vec![
        reply(200, TEXT_ANSWER),
        reply(200, TEXT_ANSWER),
    ]));
    let fixture = auth();
    let clients = TextClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let mut call = text_call(&input, &optional);
    call.timeout = Some(Duration::ZERO);
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    call.timeout = Some(Duration::from_secs(30));
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    assert_eq!(calls[0].timeout, ClientTimeout::HandlerDefault);
    assert_eq!(calls[1].timeout, thirty);
}

#[tokio::test]
async fn plans_text_async_client_omitted() {
    let optional = json("{}");
    let input = EmbeddingInput::Text(String::from("hello"));
    let mut call = text_call(&input, &optional);
    call.aembedding = true;
    let caller = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let pooled = Arc::new(PostRecorder::scripted(vec![reply(200, TEXT_ANSWER)]));
    let fixture = auth();
    let clients = TextClients {
        pooled: pooled.as_ref(),
        caller: Some(caller.as_ref()),
    };
    text_embedding(&call, &catalog(), &text_auth(&fixture), &clients)
        .await
        .unwrap();
    assert!(caller.recorded().is_empty());
    assert_eq!(pooled.recorded().len(), 1);
}

#[tokio::test]
async fn plans_multimodal_forced_headers() {
    let input = json(r#""hello""#);
    let optional = extra("{}");
    let mut headers = BTreeMap::new();
    headers.insert(String::from("Content-Type"), String::from("text/plain"));
    headers.insert(
        String::from("Authorization"),
        String::from("Bearer caller-key"),
    );
    headers.insert(String::from("x-goog-custom"), String::from("kept"));
    let call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &headers);
    let recorder = Arc::new(PostRecorder::scripted(vec![reply(200, MULTIMODAL_ANSWER)]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    let expected = BTreeMap::from([
        (String::from("Authorization"), format!("Bearer {TOKEN}")),
        (
            String::from("Content-Type"),
            String::from("application/json; charset=utf-8"),
        ),
        (String::from("x-goog-custom"), String::from("kept")),
    ]);
    assert_eq!(calls[0].headers, expected);
}

#[tokio::test]
async fn plans_multimodal_none_timeout() {
    let sync_none = ClientTimeout::Explicit(HANDLER_DEFAULT_TIMEOUT);
    assert_eq!(multimodal_timeout(None, sync_none), sync_none);
    assert_eq!(
        multimodal_timeout(None, ClientTimeout::HandlerDefault),
        ClientTimeout::HandlerDefault
    );
    let twenty = ClientTimeout::Explicit(Timeout {
        total: Duration::from_secs(20),
        connect: Duration::from_secs(20),
    });
    assert_eq!(
        multimodal_timeout(Some(Duration::from_secs(20)), sync_none),
        twenty
    );
    assert_eq!(
        multimodal_timeout(Some(Duration::from_secs(20)), ClientTimeout::HandlerDefault),
        twenty
    );

    let input = json(r#""hello""#);
    let optional = extra("{}");
    let recorder = Arc::new(PostRecorder::scripted(vec![
        reply(200, MULTIMODAL_ANSWER),
        reply(200, MULTIMODAL_ANSWER),
    ]));
    let fixture = auth();
    let clients = MultimodalClients {
        pooled: recorder.as_ref(),
        caller: None,
    };
    let mut call = multimodal_call(MULTIMODAL_MODEL, &input, &optional, &NO_HEADERS);
    call.timeout = None;
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    call.aembedding = true;
    multimodal_embedding(&call, &catalog(), &fixture.cache, &clients)
        .await
        .unwrap();
    let calls = recorder.recorded();
    assert_eq!(
        calls[0].timeout,
        ClientTimeout::Explicit(HANDLER_DEFAULT_TIMEOUT)
    );
    assert_eq!(calls[1].timeout, ClientTimeout::HandlerDefault);
}

#[test]
fn plans_supported_names() {
    assert_eq!(
        MULTIMODAL_EMBEDDING_MODELS,
        ["multimodalembedding", "multimodalembedding@001"]
    );
    assert!(!MULTIMODAL_EMBEDDING_MODELS.contains(&"multimodalembedding@002"));
    assert!(!MULTIMODAL_EMBEDDING_MODELS.contains(&"MultimodalEmbedding"));
}

#[test]
fn plans_multimodal_default_timeout() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(300));
    let explicit = ClientTimeout::Explicit(Timeout {
        total: DEFAULT_TIMEOUT,
        connect: DEFAULT_TIMEOUT,
    });
    assert_eq!(
        multimodal_timeout(Some(DEFAULT_TIMEOUT), ClientTimeout::HandlerDefault),
        explicit
    );
}

use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingRequest};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::Builder as RuntimeBuilder;
use url::Url;

use crate::openai_dispatch::OpenAi;
use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter as _;

fn deployment() -> Deployment {
    Deployment {
        model: String::from("up"),
        api_base: String::from("http://127.0.0.1:9/v1"),
        endpoint: Url::parse("http://127.0.0.1:9/v1/chat/completions").expect("url"),
        api_key: Some(SecretString::from("configured-key")),
        authorization: None,
        organization: Some(String::from("org-1")),
        timeout: Duration::from_secs(30),
        max_retries: None,
        drop_params: None,
        provider_affinity_header: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn parse(request: &str) -> ChatRequest {
    ChatRequest::parse(request.as_bytes()).expect("request")
}

fn header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    headers.get(name)?.to_str().ok()
}

fn chat_prepared(given: &str, stream: bool) -> PreparedRequest {
    let operation = if stream {
        OperationRequest::Stream(Box::new(ChatOperation {
            model: String::from("up"),
            request: parse(given),
            control: ControlOptions::default(),
        }))
    } else {
        OperationRequest::Chat(Box::new(ChatOperation {
            model: String::from("up"),
            request: parse(given),
            control: ControlOptions::default(),
        }))
    };
    OpenAi
        .prepare(&operation, &deployment(), &catalog())
        .expect("prepared")
}

fn assert_body(given: &str, expected: &str) {
    let prepared = chat_prepared(given, false);
    let PreparedRequest::Chat(chat) = &prepared else {
        panic!("a buffered chat prepares the chat request");
    };
    let body: JsonValue = serde_json::from_slice(&chat.http.body).expect("body");
    let wanted: JsonValue = serde_json::from_str(expected).expect("expected");
    assert_eq!(body, wanted);
}

fn plain_chat() {
    assert_body(
        r#"{"model":"g","messages":[{"role":"user","content":"hi"}]}"#,
        r#"{"model":"up","messages":[{"role":"user","content":"hi"}]}"#,
    );
    let prepared = chat_prepared(r#"{"model":"g","messages":[]}"#, false);
    let PreparedRequest::Chat(chat) = &prepared else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(
        chat.http.url.as_str(),
        "http://127.0.0.1:9/v1/chat/completions"
    );
    assert_eq!(
        header(&chat.http.headers, "accept"),
        Some("application/json")
    );
    assert_eq!(
        header(&chat.http.headers, "openai-organization"),
        Some("org-1")
    );
    assert_eq!(
        header(&chat.http.headers, "authorization"),
        Some("Bearer configured-key")
    );
}

fn mapped_params() {
    assert_body(
        r#"{"model":"g","messages":[],"temperature":0.2,"top_p":0.9}"#,
        r#"{"model":"up","messages":[],"temperature":0.2,"top_p":0.9}"#,
    );
}

fn forwarded_params() {
    assert_body(
        r#"{"model":"g","messages":[],"custom_field":7}"#,
        r#"{"model":"up","messages":[],"custom_field":7}"#,
    );
}

fn control_not_forwarded() {
    assert_body(
        r#"{"model":"g","messages":[],"stream_chunk_size":4}"#,
        r#"{"model":"up","messages":[]}"#,
    );
}

fn stream_request() {
    let prepared = chat_prepared(
        r#"{"model":"g","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
        true,
    );
    let PreparedRequest::Stream(stream) = &prepared else {
        panic!("a stream request prepares the stream request");
    };
    let body: JsonValue = serde_json::from_slice(&stream.http.body).expect("body");
    let wanted: JsonValue = serde_json::from_str(
        r#"{"model":"up","messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .expect("expected");
    assert_eq!(body, wanted);
    assert_eq!(
        header(&stream.http.headers, "accept"),
        Some("text/event-stream")
    );
}

fn embedding_request() {
    let request = EmbeddingRequest {
        model: Some(String::from("client-embed")),
        input: EmbeddingInput::Text(String::from("hello")),
        encoding_format: None,
        dimensions: None,
        user: None,
        passthrough: IndexMap::new(),
    };
    let operation = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("text-embedding-3-small"),
        request,
    }));
    let prepared = OpenAi
        .prepare(&operation, &deployment(), &catalog())
        .expect("prepared");
    let PreparedRequest::Embedding(embedding) = &prepared else {
        panic!("an embedding request prepares the embedding request");
    };
    assert_eq!(
        embedding.http.url.as_str(),
        "http://127.0.0.1:9/v1/embeddings"
    );
    let body: JsonValue = serde_json::from_slice(&embedding.http.body).expect("body");
    let wanted: JsonValue =
        serde_json::from_str(r#"{"input":"hello","model":"text-embedding-3-small"}"#)
            .expect("expected");
    assert_eq!(body, wanted);
}

fn answer(status: u16, body: &str) -> Upstream {
    Upstream::Buffered(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    })
}

fn decode_chat() {
    let prepared = chat_prepared(r#"{"model":"g","messages":[]}"#, false);
    let decoded = OpenAi
        .decode(
            &prepared,
            answer(
                200,
                r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
            ),
        )
        .expect("decoded");
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = decoded else {
        panic!("the openai projection decodes to a chat completion");
    };
    assert_eq!(completion.model, "g");
    assert_eq!(completion.usage.total_tokens, 3);
}

fn decode_error() {
    let prepared = chat_prepared(r#"{"model":"g","messages":[]}"#, false);
    let refused = match OpenAi.decode(&prepared, answer(429, r#"{"error":{"message":"slow"}}"#)) {
        Ok(_answer) => panic!("an error status is a provider error"),
        Err(error) => error,
    };
    if let OperationError::Provider { response } = refused {
        assert_eq!(response.status, 429);
    } else {
        panic!("a provider status turns into a provider error");
    }
}

fn the_debug_text_hides_the_key() {
    let shown = format!(
        "{:?}",
        chat_prepared(r#"{"model":"g","messages":[]}"#, false)
    );
    assert!(!shown.contains("configured-key"));
    assert!(shown.contains("Sensitive"));
}

fn parallel_prepares_do_not_share_state() {
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (first, second) = runtime.block_on(async {
        tokio::join!(
            async {
                chat_prepared(
                    r#"{"model":"client-a","messages":[{"role":"user","content":"a"}]}"#,
                    false,
                )
            },
            async {
                chat_prepared(
                    r#"{"model":"client-b","messages":[{"role":"user","content":"b"}]}"#,
                    false,
                )
            },
        )
    });
    let PreparedRequest::Chat(first_chat) = &first else {
        panic!("the first request prepares a chat");
    };
    let PreparedRequest::Chat(second_chat) = &second else {
        panic!("the second request prepares a chat");
    };
    assert_eq!(first_chat.client_model.as_deref(), Some("client-a"));
    assert_eq!(second_chat.client_model.as_deref(), Some("client-b"));
    assert_ne!(first_chat.http.body, second_chat.http.body);
}

#[rstest]
#[case::plain_chat(plain_chat)]
#[case::mapped_params(mapped_params)]
#[case::forwarded_params(forwarded_params)]
#[case::control_not_forwarded(control_not_forwarded)]
#[case::stream_request(stream_request)]
#[case::embedding_request(embedding_request)]
#[case::decode_chat(decode_chat)]
#[case::decode_error(decode_error)]
#[case::debug_hides_key(the_debug_text_hides_the_key)]
#[case::parallel_prepares(parallel_prepares_do_not_share_state)]
fn provider_framework_tests_behavior(#[case] check: fn()) {
    check();
}

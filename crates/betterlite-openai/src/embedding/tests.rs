#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use crate::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use crate::chat::execution::ChatExecutor;
use crate::openai_base_config::CatalogCapabilities;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    EmbeddingError, EmbeddingInput, EmbeddingRequest, EmbeddingVector, EncodingFormat, servable,
};

#[test]
fn parses_embedding_response_vectors() {
    let floats: EmbeddingVector = serde_json::from_str("[0.1,0.2]").expect("float vector");
    let base64: EmbeddingVector = serde_json::from_str("\"AQID\"").expect("base64 vector");
    assert!(matches!(floats, EmbeddingVector::Floats(values) if values.len() == 2));
    assert!(matches!(base64, EmbeddingVector::Base64(value) if value == "AQID"));
}

#[test]
fn parses_embedding_string_input() {
    let request =
        EmbeddingRequest::parse(br#"{"model":"m","input":"hello","encoding_format":"base64"}"#)
            .expect("embedding request");
    assert_eq!(request.input, EmbeddingInput::Text(String::from("hello")));
    assert_eq!(request.encoding_format, Some(EncodingFormat::Base64));
}

#[test]
fn parses_embedding_arrays_and_tokens() {
    let strings = EmbeddingRequest::parse(br#"{"input":["a","b"]}"#).expect("string array request");
    assert_eq!(
        strings.input,
        EmbeddingInput::Texts(vec![String::from("a"), String::from("b")])
    );
    let tokens = EmbeddingRequest::parse(br#"{"input":[[1,2],[3]]}"#).expect("token array request");
    assert_eq!(
        tokens.input,
        EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]])
    );
}

#[test]
fn rejects_embedding_deployments_without_openai_support_or_keys() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = loader
        .load(
            "model_list:\n  - model_name: unsupported\n    litellm_params: {model: anthropic/claude}\n  - model_name: missing-key\n    litellm_params: {model: openai/embed}\n",
        )
        .expect("embedding deployment config");
    let deployments: Vec<ChatDeployment> = config
        .model_list
        .iter()
        .map(|entry| {
            ChatDeployment::resolve(entry, &OpenAiDefaults::default()).expect("deployment")
        })
        .collect();
    assert!(matches!(
        servable(&deployments[0]),
        Err(EmbeddingError::UnsupportedProvider { .. })
    ));
    assert!(matches!(
        servable(&deployments[1]),
        Err(EmbeddingError::MissingApiKey)
    ));
}

#[test]
fn accepts_embedding_openai_deployment_with_key() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "model_list:\n  - model_name: embed\n    litellm_params: {model: openai/embed, api_key: key}\n",
    )
    .expect("embedding deployment config");
    let deployment = ChatDeployment::resolve(
        config.model_list.first().expect("deployment"),
        &OpenAiDefaults::default(),
    )
    .expect("deployment");
    let (model, _) = servable(&deployment).expect("servable embedding");
    assert_eq!(model, "embed");
    assert!(matches!(deployment.provider, Provider::OpenAi { .. }));
}

#[tokio::test]
async fn reports_embedding_provider_failures() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("connection");
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
            )
            .await
            .expect("response");
    });
    let (executor, deployment) = embedding_executor(&base);
    let request = EmbeddingRequest::parse(br#"{"input":"hello"}"#).expect("request");
    let result = super::send(&executor, &deployment, &request).await;
    assert!(matches!(result, Err(EmbeddingError::Provider { .. })));
    tasks
        .join_next()
        .await
        .expect("responder")
        .expect("responder");
}

fn embedding_executor(base: &str) -> (ChatExecutor, ChatDeployment) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!(
        "model_list:\n  - model_name: embed\n    litellm_params: {{model: openai/embed, api_base: {base}, api_key: key}}\n"
    ))
    .expect("config");
    let deployment = ChatDeployment::resolve(
        config.model_list.first().expect("deployment"),
        &OpenAiDefaults::default(),
    )
    .expect("deployment");
    let policy = Arc::new(ProxyPolicy::resolve(
        &ProxyTrust::default(),
        &BTreeMap::new(),
    ));
    let executor = ChatExecutor {
        client: ProxiedClient::outbound(&policy).expect("client"),
        deployments: vec![deployment.clone()],
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(1)),
    };
    (executor, deployment)
}

#[test]
fn embedding_post_preserves_fields() {
    let (_, deployment) = embedding_executor("https://provider.test/v1/");
    let request = EmbeddingRequest::parse(
        br#"{"model":"alias","input":[1,2],"dimensions":2,"user":"reader","encoding_format":"float"}"#,
    )
    .expect("request");
    let (model, key) = servable(&deployment).expect("deployment");
    let post = super::post(&deployment, &request, model, key).expect("post");
    assert_eq!(post.url.as_str(), "https://provider.test/v1/embeddings");
    let body: EmbeddingRequest = serde_json::from_slice(&post.body).expect("upstream body");
    assert_eq!(body.model.as_deref(), Some("embed"));
    assert_eq!(body.input, request.input);
    assert_eq!(body.dimensions, Some(2));
    assert_eq!(body.user.as_deref(), Some("reader"));
    assert_eq!(body.encoding_format, Some(EncodingFormat::Float));
}

#[rstest::rstest]
#[case::alias(Some("alias"), "alias")]
#[case::provider(None, "upstream")]
fn embedding_response_model_mapping(#[case] requested: Option<&str>, #[case] expected: &str) {
    let response: super::EmbeddingResponse = serde_json::from_str(
        r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25,-1]}],"model":"upstream","usage":{"prompt_tokens":2,"total_tokens":2}}"#,
    )
    .expect("response");
    let mapped = response.clone().for_client(requested);
    assert_eq!(mapped.model, expected);
    assert_eq!(mapped.data, response.data);
    assert_eq!(mapped.usage, response.usage);
    assert_eq!(mapped.object, "list");
    assert_eq!(mapped.usage.completion_tokens, 0);
    assert_eq!(mapped.usage.completion_tokens_details, None);
    assert_eq!(mapped.usage.prompt_tokens_details, None);
}

#[rstest::rstest]
fn replays_the_embedding_corpus(#[files("fuzz/corpus/embedding/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let _request = EmbeddingRequest::parse(&bytes);
    let _response = serde_json::from_slice::<super::EmbeddingResponse>(&bytes);
}

#[rstest::rstest]
#[case::malformed("{")]
#[case::missing_input(r#"{"model":"embed"}"#)]
#[case::mixed_input(r#"{"input":["text",2]}"#)]
#[case::negative_token(r#"{"input":[-1]}"#)]
#[case::overflow_token(r#"{"input":[4294967296]}"#)]
#[case::bad_encoding(r#"{"input":"text","encoding_format":"binary"}"#)]
fn rejects_invalid_embedding_input(#[case] body: &str) {
    assert!(matches!(
        EmbeddingRequest::parse(body.as_bytes()),
        Err(EmbeddingError::Body { .. })
    ));
}

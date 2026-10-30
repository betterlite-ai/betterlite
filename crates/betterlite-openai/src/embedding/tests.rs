#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

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

use super::{EmbeddingError, EmbeddingInput, EmbeddingRequest, EncodingFormat, servable};

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

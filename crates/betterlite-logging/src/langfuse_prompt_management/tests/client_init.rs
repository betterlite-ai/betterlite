use std::sync::Arc;

use super::support;
use crate::langfuse_prompt_management::clients::{
    LangfusePromptClients, PROMPT_CLIENT_CACHE_CAPACITY, PromptClientRequest,
};

fn request<'request>(
    public_key: &'request str,
    host: &'request str,
) -> PromptClientRequest<'request> {
    PromptClientRequest {
        public_key: Some(public_key),
        secret: Some("sk-1"),
        secret_key: None,
        host: Some(host),
        flush_interval: 1,
        allow_env_credentials: false,
    }
}

#[tokio::test]
async fn identical_credentials_and_host_share_one_client() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env(support::HOST);
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let first = clients
        .client_init(&services, &request("pk-1", support::HOST))
        .expect("the client builds");
    let second = clients
        .client_init(&services, &request("pk-1", support::HOST))
        .expect("the cached client comes back");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(builder.configs().len(), 1);
}

#[tokio::test]
async fn every_one_of_the_six_arguments_separates_the_cache() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env(support::HOST);
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let base = request("pk-1", support::HOST);
    let variants = [
        PromptClientRequest {
            public_key: Some("pk-2"),
            ..base
        },
        PromptClientRequest {
            secret: Some("sk-2"),
            ..base
        },
        PromptClientRequest {
            secret_key: Some("sk-3"),
            ..base
        },
        PromptClientRequest {
            host: Some("http://other.test"),
            ..base
        },
        PromptClientRequest {
            flush_interval: 2,
            ..base
        },
        PromptClientRequest {
            allow_env_credentials: true,
            ..base
        },
    ];
    for variant in variants {
        clients
            .client_init(&services, &variant)
            .expect("the variant builds");
    }
    clients
        .client_init(&services, &base)
        .expect("the base builds");
    assert_eq!(builder.configs().len(), 7);
}

#[tokio::test]
async fn the_eleventh_client_evicts_the_least_recently_used_one() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env(support::HOST);
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let keys: Vec<String> = (0..=PROMPT_CLIENT_CACHE_CAPACITY)
        .map(|index| format!("pk-{index}"))
        .collect();
    for key in &keys {
        clients
            .client_init(&services, &request(key.as_str(), support::HOST))
            .expect("the client builds");
    }
    assert_eq!(builder.configs().len(), PROMPT_CLIENT_CACHE_CAPACITY + 1);
    clients
        .client_init(&services, &request(keys[0].as_str(), support::HOST))
        .expect("the evicted client rebuilds");
    assert_eq!(builder.configs().len(), PROMPT_CLIENT_CACHE_CAPACITY + 2);
    clients
        .client_init(
            &services,
            &request(keys[PROMPT_CLIENT_CACHE_CAPACITY].as_str(), support::HOST),
        )
        .expect("the newest client is still cached");
    assert_eq!(builder.configs().len(), PROMPT_CLIENT_CACHE_CAPACITY + 2);
}

#[tokio::test]
async fn reading_a_cached_client_makes_it_the_most_recently_used() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env(support::HOST);
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let clients = LangfusePromptClients::default();
    let keys: Vec<String> = (0..PROMPT_CLIENT_CACHE_CAPACITY)
        .map(|index| format!("pk-{index}"))
        .collect();
    for key in &keys {
        clients
            .client_init(&services, &request(key.as_str(), support::HOST))
            .expect("the client builds");
    }
    clients
        .client_init(&services, &request(keys[0].as_str(), support::HOST))
        .expect("the cached client is touched");
    clients
        .client_init(&services, &request("pk-new", support::HOST))
        .expect("the eleventh client builds");
    clients
        .client_init(&services, &request(keys[0].as_str(), support::HOST))
        .expect("the touched client is still cached");
    assert_eq!(builder.configs().len(), PROMPT_CLIENT_CACHE_CAPACITY + 1);
    clients
        .client_init(&services, &request(keys[1].as_str(), support::HOST))
        .expect("the untouched client was evicted");
    assert_eq!(builder.configs().len(), PROMPT_CLIENT_CACHE_CAPACITY + 2);
}

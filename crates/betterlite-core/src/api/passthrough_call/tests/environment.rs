#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::openai_base_config::Object;

use super::super::auth;
use super::super::config::PassthroughCallConfig;
use super::super::{PassthroughCallArgs, allm_passthrough_route};
use super::support::{base_args, logging_state, proxy_client, test_environment};

fn args_for(model: &str) -> PassthroughCallArgs {
    let mut args = base_args();
    args.model = String::from(model);
    args.custom_llm_provider = None;
    args
}

fn azure_args(api_version: &str) -> PassthroughCallArgs {
    let mut args = args_for("azure/my-deployment");
    args.extra.insert(
        "api_version".to_owned(),
        JsonValue::String(api_version.to_owned()),
    );
    args
}

#[tokio::test]
async fn gigachat_reads_its_base_and_access_token_from_the_environment() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}/api/v1", serving.address);
    let environment = test_environment(&[
        ("GIGACHAT_API_BASE", base.as_str()),
        ("GIGACHAT_ACCESS_TOKEN", "env-token"),
    ]);
    let answer = allm_passthrough_route(
        args_for("gigachat/GigaChat"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    assert_eq!(answer.status, 200);
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /api/v1/chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer env-token"));
}

#[tokio::test]
async fn gigachat_mints_its_token_from_the_auth_url_environment() {
    let upstream = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let auth_server = serve(
        reply(
            "200 fixture",
            &[],
            r#"{"tok":"fetched-token","exp":4102444800000}"#,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let base = format!("http://{}/api/v1", upstream.address);
    let auth_url = format!("http://{}/api/v2/oauth", auth_server.address);
    let environment = test_environment(&[
        ("GIGACHAT_API_BASE", base.as_str()),
        ("GIGACHAT_CREDENTIALS", "client-secret"),
        ("GIGACHAT_AUTH_URL", auth_url.as_str()),
    ]);
    allm_passthrough_route(
        args_for("gigachat/GigaChat"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = upstream.finish().unwrap();
    assert_eq!(sent.line, "POST /api/v1/chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer fetched-token"));
    let token_request = auth_server.finish().unwrap();
    assert_eq!(token_request.line, "POST /api/v2/oauth HTTP/1.1");
    assert!(
        token_request
            .head
            .contains("authorization: Basic client-secret")
    );
    assert_eq!(token_request.body, "scope=GIGACHAT_API_PERS");
}

#[tokio::test]
async fn nvidia_nim_reads_its_base_and_key_from_the_environment() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}/v1", serving.address);
    let environment = test_environment(&[
        ("NVIDIA_NIM_API_BASE", base.as_str()),
        ("NVIDIA_NIM_API_KEY", "nim-env-key"),
    ]);
    allm_passthrough_route(
        args_for("nvidia_nim/meta/llama3-8b-instruct"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer nim-env-key"));
}

#[tokio::test]
async fn azure_reads_its_base_and_key_from_the_environment() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let environment = test_environment(&[
        ("AZURE_API_BASE", base.as_str()),
        ("AZURE_API_KEY", "azure-env-key"),
    ]);
    allm_passthrough_route(
        azure_args("2024-02-01"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert_eq!(
        sent.line,
        "POST /chat/completions?api-version=2024-02-01 HTTP/1.1"
    );
    assert!(sent.head.contains("api-key: azure-env-key"));
}

#[tokio::test]
async fn azure_openai_key_reaches_the_request_when_azure_api_key_is_unset() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let mut args = azure_args("2024-02-01");
    args.api_base = Some(format!("http://{}", serving.address));
    let environment = test_environment(&[("AZURE_OPENAI_API_KEY", "openai-env-key")]);
    allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("api-key: openai-env-key"));
}

#[tokio::test]
async fn azure_api_key_beats_the_azure_openai_key() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let environment = test_environment(&[
        ("AZURE_API_BASE", base.as_str()),
        ("AZURE_OPENAI_API_KEY", "openai-env-key"),
        ("AZURE_API_KEY", "azure-env-key"),
    ]);
    allm_passthrough_route(
        azure_args("2024-02-01"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("api-key: azure-env-key"));
    assert!(!sent.head.contains("openai-env-key"));
}

#[tokio::test]
async fn azure_ad_token_becomes_the_bearer_header() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let environment = test_environment(&[
        ("AZURE_API_BASE", base.as_str()),
        ("AZURE_AD_TOKEN", "ad-token"),
    ]);
    allm_passthrough_route(
        azure_args("2024-02-01"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("authorization: Bearer ad-token"));
}

#[tokio::test]
async fn azure_param_ad_token_becomes_the_bearer_header() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let environment = test_environment(&[("AZURE_API_BASE", base.as_str())]);
    let mut args = azure_args("2024-02-01");
    args.extra.insert(
        "azure_ad_token".to_owned(),
        JsonValue::String(String::from("param-token")),
    );
    allm_passthrough_route(
        args,
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert!(sent.head.contains("authorization: Bearer param-token"));
}

#[tokio::test]
async fn azure_ai_entra_reads_the_ad_token() {
    let config = PassthroughCallConfig {
        tag: PassthroughConfigTag::AzureAI,
        environment: Arc::new(test_environment(&[("AZURE_AD_TOKEN", "azure-ai-ad-token")])),
    };
    let headers = auth::validate(
        &config,
        None,
        &Object::new(),
        "https://example.test",
        &proxy_client(1024),
    )
    .await
    .unwrap();
    assert_eq!(
        headers.get("Authorization"),
        Some(&String::from("Bearer azure-ai-ad-token"))
    );
}

#[tokio::test]
async fn hosted_vllm_reads_its_base_from_the_environment() {
    let serving = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let base = format!("http://{}/v1", serving.address);
    let environment = test_environment(&[
        ("HOSTED_VLLM_API_BASE", base.as_str()),
        ("HOSTED_VLLM_API_KEY", "vllm-env-key"),
    ]);
    allm_passthrough_route(
        args_for("hosted_vllm/meta-llama/Llama-3-8B"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = serving.finish().unwrap();
    assert_eq!(sent.line, "POST /v1/chat/completions HTTP/1.1");
}

#[tokio::test]
async fn watsonx_reads_its_url_key_and_iam_url_from_the_environment() {
    let upstream = serve(reply("200 fixture", &[], "pong"), Duration::ZERO).unwrap();
    let iam = serve(
        reply(
            "200 fixture",
            &[],
            r#"{"access_token":"iam-token","expires_in":3600}"#,
        ),
        Duration::ZERO,
    )
    .unwrap();
    let base = format!("http://{}/ml", upstream.address);
    let iam_url = format!("http://{}/identity/token", iam.address);
    let environment = test_environment(&[
        ("WATSONX_URL", base.as_str()),
        ("WATSONX_APIKEY", "watsonx-env-key"),
        ("WATSONX_IAM_URL", iam_url.as_str()),
    ]);
    allm_passthrough_route(
        args_for("watsonx/ibm/granite-13b-chat-v2"),
        &mut logging_state(),
        &proxy_client(1024),
        &environment,
    )
    .await
    .expect("the upstream answers");
    let sent = upstream.finish().unwrap();
    assert_eq!(sent.line, "POST /ml/chat/completions HTTP/1.1");
    assert!(sent.head.contains("authorization: Bearer iam-token"));
    let token_request = iam.finish().unwrap();
    assert_eq!(token_request.line, "POST /identity/token HTTP/1.1");
    assert!(token_request.body.contains("apikey=watsonx-env-key"));
}

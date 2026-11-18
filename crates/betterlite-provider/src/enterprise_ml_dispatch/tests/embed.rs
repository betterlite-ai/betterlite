use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::enterprise_ml_dispatch::embed::{self, EmbeddingContext, EmbeddingReply};
use crate::enterprise_ml_dispatch::error::EnterpriseError;
use crate::enterprise_ml_dispatch::sap::{self, SapRequest};
use crate::enterprise_ml_dispatch::select::{
    EmbeddingConfig, RerankConfig, embedding_config, rerank_config,
};
use crate::sap_auth::error::SapAuthError;
use crate::sap_chat::error::SapChatError;
use crate::sap_embedding::error::SapEmbeddingError;
use crate::watsonx_embed::error::WatsonxEmbeddingError;

use super::support::{
    Env, FixedClock, SAP_CHAT_REPLIES, SAP_EMBEDDING_REPLIES, SAP_SERVICE_KEY, Sagemaker, Scripted,
    at, dispatch_embedding, embedding_call, key, sap_credentials_environment, sap_environment,
};

#[test]
fn embedding_configuration_picks_the_three_arms() {
    assert_eq!(
        embedding_config(ProviderId::SapGenerativeAiHub),
        Some(EmbeddingConfig::Sap)
    );
    assert_eq!(
        embedding_config(ProviderId::Watsonx),
        Some(EmbeddingConfig::Watsonx)
    );
    assert_eq!(
        embedding_config(ProviderId::Sagemaker),
        Some(EmbeddingConfig::Sagemaker)
    );
    assert_eq!(embedding_config(ProviderId::Openai), None);
}

#[test]
fn rerank_configuration_picks_watsonx_only() {
    assert_eq!(
        rerank_config(ProviderId::Watsonx),
        Some(RerankConfig::Watsonx)
    );
    assert_eq!(rerank_config(ProviderId::Sagemaker), None);
    assert_eq!(rerank_config(ProviderId::Openai), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_runs_the_token_and_deployment_flow() {
    let transport = Scripted::new(SAP_EMBEDDING_REPLIES.to_vec());
    let env = Env::new(sap_credentials_environment());
    let call = embedding_call("text-embedding-3-small", Some(key(SAP_SERVICE_KEY)));
    let reply = dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env)
        .await
        .expect("the sap embedding resolves");
    let EmbeddingReply::Sap(response) = reply else {
        panic!("a sap reply is expected");
    };
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.prompt_tokens, 2);
    let sent = transport.sent();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0].url, "https://auth.example.com/oauth/token");
    assert_eq!(sent[1].url, "https://base.example.com/v2/lm/deployments");
    assert_eq!(sent[4].url, "https://dep.example.com/v2/embeddings");
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_sends_the_orchestration_headers() {
    let transport = Scripted::new(SAP_EMBEDDING_REPLIES.to_vec());
    let env = Env::new(sap_credentials_environment());
    let call = embedding_call("text-embedding-3-small", Some(key(SAP_SERVICE_KEY)));
    dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env)
        .await
        .expect("the sap embedding resolves");
    let sent = transport.sent();
    assert_eq!(
        String::from_utf8(sent[0].body.clone()).expect("the form is text"),
        "grant_type=client_credentials&client_id=client&client_secret=secret"
    );
    assert_sap_headers(&transport);
}

fn assert_sap_headers(transport: &Scripted) {
    for index in 1..=4 {
        let auth = transport.header(index, http::header::AUTHORIZATION);
        assert_eq!(auth.as_deref(), Some("Bearer oracle-token"));
        assert_eq!(resource_group(transport, index).as_deref(), Some("default"));
        assert_eq!(client_type(transport, index).as_deref(), Some("LiteLLM"));
    }
    let sent = transport.sent();
    for request in &sent[..5] {
        assert_eq!(request.deadline, Duration::from_secs(30));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_sends_the_orchestration_body() {
    let transport = Scripted::new(SAP_EMBEDDING_REPLIES.to_vec());
    let env = Env::new(sap_credentials_environment());
    let call = embedding_call("text-embedding-3-small", Some(key(SAP_SERVICE_KEY)));
    dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env)
        .await
        .expect("the sap embedding resolves");
    let sent = transport.sent();
    assert_eq!(
        sent[2].url,
        "https://base.example.com/v2/lm/configurations/old"
    );
    assert_eq!(
        sent[3].url,
        "https://base.example.com/v2/lm/configurations/new"
    );
    let body = transport.body(4);
    assert_eq!(
        at(&body, &["input", "text"]),
        &Json::Text(String::from("hi"))
    );
    assert_eq!(
        at(&body, &["config", "modules", "embeddings", "model", "name"]),
        &Json::Text(String::from("text-embedding-3-small"))
    );
    assert_eq!(
        at(
            &body,
            &["config", "modules", "embeddings", "model", "version"]
        ),
        &Json::Text(String::from("latest"))
    );
}

fn resource_group(transport: &Scripted, index: usize) -> Option<String> {
    transport.header(
        index,
        http::header::HeaderName::from_static("ai-resource-group"),
    )
}

fn client_type(transport: &Scripted, index: usize) -> Option<String> {
    transport.header(
        index,
        http::header::HeaderName::from_static("ai-client-type"),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_chat_prepare_runs_the_chat_deployment_lookup() {
    let transport = Scripted::new(SAP_CHAT_REPLIES.to_vec());
    let env = Env::new(sap_credentials_environment());
    let request = SapRequest {
        api_key: None,
        sap_service_key: None,
        env: env.source(),
    };
    let prepared =
        sap::prepare_chat(request, &transport, &FixedClock).expect("the chat session resolves");
    // the chat config appends the path as LiteLLM does, so a trailing slash in the deployment URL stays
    assert_eq!(prepared.url, "https://dep.example.com//v2/completion");
    assert_eq!(
        prepared
            .headers
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer oracle-token")
    );
    assert_eq!(transport.sent().len(), 3);

    let empty = Env::new(sap_environment(&[]));
    let request = SapRequest {
        api_key: None,
        sap_service_key: None,
        env: empty.source(),
    };
    let error =
        sap::prepare_chat(request, &transport, &FixedClock).expect_err("no credentials are set");
    assert!(matches!(
        error,
        EnterpriseError::SapChat(SapChatError::Orchestration(_))
    ));
    assert_eq!(error.status(), Some(400));
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_service_key_supplies_the_credentials() {
    let transport = Scripted::new(SAP_CHAT_REPLIES.to_vec());
    let env = Env::new(sap_environment(&[]));
    let service_key = key(SAP_SERVICE_KEY);
    let request = SapRequest {
        api_key: None,
        sap_service_key: Some(&service_key),
        env: env.source(),
    };
    let prepared =
        sap::prepare_chat(request, &transport, &FixedClock).expect("the service key resolves");
    assert_eq!(prepared.url, "https://dep.example.com//v2/completion");
    let sent = transport.sent();
    assert_eq!(sent[0].url, "https://other-auth.example.com/oauth/token");
    assert_eq!(
        sent[1].url,
        "https://other-base.example.com/v2/lm/deployments"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_reports_missing_credentials() {
    let transport = Scripted::empty();
    let env = Env::new(sap_environment(&[]));
    let call = embedding_call("text-embedding-3-small", None);
    let outcome = dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env).await;
    let Err(error) = outcome else {
        panic!("no credentials are set");
    };
    assert!(matches!(
        error,
        EnterpriseError::SapEmbedding(SapEmbeddingError::Auth(SapAuthError::NoCredentials))
    ));
    assert_eq!(error.status(), None);
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_refuses_a_client_certificate() {
    let transport = Scripted::empty();
    let env = Env::new(sap_environment(&[
        ("AICORE_CLIENT_ID", "client"),
        ("AICORE_AUTH_URL", "https://auth.example.com"),
        ("AICORE_BASE_URL", "https://base.example.com"),
        ("AICORE_CERT_STR", "cert"),
        ("AICORE_KEY_STR", "key"),
    ]));
    let call = embedding_call("text-embedding-3-small", None);
    let outcome = dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env).await;
    let Err(error) = outcome else {
        panic!("the pooled client holds no certificate");
    };
    assert!(
        error.to_string().contains(
            "client certificate authentication is not supported by the pooled SAP client"
        )
    );
    assert_eq!(error.status(), None);
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_reports_a_non_success_reply() {
    let mut replies = SAP_EMBEDDING_REPLIES.to_vec();
    replies[4] = (400, r#"{"error":"bad"}"#);
    let transport = Scripted::new(replies);
    let env = Env::new(sap_credentials_environment());
    let call = embedding_call("text-embedding-3-small", None);
    let outcome = dispatch_embedding(&call, EmbeddingConfig::Sap, &transport, &env).await;
    let Err(error) = outcome else {
        panic!("the orchestration service rejects the embedding");
    };
    assert!(matches!(error, EnterpriseError::SapOrchestration(_)));
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.to_string(), r#"{"error":"bad"}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_posts_the_project_request() {
    let transport = Scripted::new(vec![(
        200,
        r#"{"results":[{"embedding":[0.1,0.2]}],"input_token_count":2}"#,
    )]);
    let env = Env::new(BTreeMap::from([
        (String::from("WATSONX_TOKEN"), String::from("env-token")),
        (String::from("WATSONX_PROJECT_ID"), String::from("project")),
    ]));
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let reply = dispatch_embedding(&call, EmbeddingConfig::Watsonx, &transport, &env)
        .await
        .expect("the watsonx embedding resolves");
    let EmbeddingReply::Watsonx(response) = reply else {
        panic!("a watsonx reply is expected");
    };
    assert_eq!(response.model, "ibm/granite-embedding");
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].url,
        "https://wx.example.com/ml/v1/text/embeddings?version=2024-03-13"
    );
    assert_eq!(
        transport.header(0, http::header::AUTHORIZATION).as_deref(),
        Some("Bearer env-token")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_sends_the_generation_body() {
    let transport = Scripted::new(vec![(
        200,
        r#"{"results":[{"embedding":[0.1,0.2]}],"input_token_count":2}"#,
    )]);
    let env = Env::new(BTreeMap::from([
        (String::from("WATSONX_TOKEN"), String::from("env-token")),
        (String::from("WATSONX_PROJECT_ID"), String::from("project")),
    ]));
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    dispatch_embedding(&call, EmbeddingConfig::Watsonx, &transport, &env)
        .await
        .expect("the watsonx embedding resolves");
    let body = transport.body(0);
    assert_eq!(
        at(&body, &["inputs"]),
        &Json::List(vec![Json::Text(String::from("hi"))])
    );
    // the credentials hold no token, and LiteLLM still copies the key into the parameters
    let parameters = Object::from([(String::from("token"), Json::Null)]);
    assert_eq!(at(&body, &["parameters"]), &Json::Object(parameters));
    assert_eq!(
        at(&body, &["model_id"]),
        &Json::Text(String::from("ibm/granite-embedding"))
    );
    assert_eq!(
        at(&body, &["project_id"]),
        &Json::Text(String::from("project"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_without_ids_reports_401() {
    let transport = Scripted::empty();
    let env = Env::new(BTreeMap::from([(
        String::from("WATSONX_TOKEN"),
        String::from("env-token"),
    )]));
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let outcome = dispatch_embedding(&call, EmbeddingConfig::Watsonx, &transport, &env).await;
    let Err(error) = outcome else {
        panic!("no project or space is set");
    };
    assert!(matches!(
        error,
        EnterpriseError::WatsonxEmbedding(WatsonxEmbeddingError::Watsonx(ref inner))
            if matches!(inner.as_ref(), WatsonxError::Api { status_code: 401, .. })
    ));
    assert_eq!(error.status(), Some(401));
    assert!(
        error
            .to_string()
            .contains("project_id and space_id not set")
    );
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_reports_a_non_success_reply() {
    let transport = Scripted::new(vec![(400, r#"{"errors":[{"code":"bad"}]}"#)]);
    let env = Env::new(BTreeMap::from([
        (String::from("WATSONX_TOKEN"), String::from("env-token")),
        (String::from("WATSONX_PROJECT_ID"), String::from("project")),
    ]));
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let outcome = dispatch_embedding(&call, EmbeddingConfig::Watsonx, &transport, &env).await;
    let Err(error) = outcome else {
        panic!("the embedding service rejects the request");
    };
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.to_string(), r#"{"errors":[{"code":"bad"}]}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn sagemaker_embedding_posts_the_hf_request() {
    let sagemaker = Sagemaker::new(Scripted::new(vec![(200, "[[0.1,0.2]]")]));
    let unused = Scripted::empty();
    let env = Env::new(BTreeMap::new());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env: env.source(),
        sap_service_key: None,
        transport: &unused,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("sagemaker/endpoint", None);
    let reply = embed::dispatch(&call, EmbeddingConfig::Sagemaker, &context)
        .await
        .expect("the sagemaker embedding resolves");
    assert!(matches!(reply, EmbeddingReply::Sagemaker(_)));
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/endpoint/invocations"
    );
    assert!(sent[0].headers.get(http::header::AUTHORIZATION).is_some());
    assert_eq!(
        at(&sagemaker.transport.body(0), &["inputs"]),
        &Json::List(vec![Json::Text(String::from("hi"))])
    );
}

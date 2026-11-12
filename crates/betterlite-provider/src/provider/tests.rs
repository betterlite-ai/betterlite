#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;

use super::media_providers::MediaProviders;
use super::speech_providers::SpeechProviders;
use super::subscription::Subscription;
use crate::anthropic_dispatch::Anthropic;
use crate::azure_dispatch::Azure;
use crate::bedrock_dispatch_contract::Bedrock;
use crate::compatible_providers::Compatible;
use crate::embedding_rerank_providers::EmbeddingRerank;
use crate::foundry_dispatch::AzureAi;
use crate::legacy_dispatch::Legacy;
use crate::openai_dispatch::OpenAi;
use crate::operation::{ChatOperation, OperationError, OperationRequest};
use crate::prepared::Upstream;
use crate::prepared::deployment::Deployment;
use crate::provider::Provider;
use crate::self_hosted_dispatch::SelfHosted;
use crate::studio_dispatch::Gemini;
use crate::vertex_dispatch::VertexAi;
use crate::xai_mistral::XaiMistral;

fn listed(model: &str, api_base: &str) -> ChatDeployment {
    ChatDeployment::resolve(
        &ModelDeployment {
            model_name: String::from("g"),
            litellm_params: LiteLlmParams {
                ssl_verify: None,
                model: Some(model.to_owned()),
                api_base: Some(api_base.to_owned()),
                api_key: Some(SecretString::from("configured-key")),
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            },
            model_info: ModelInfo::default(),
        },
        &OpenAiDefaults::default(),
    )
    .expect("deployment")
}

fn deployment() -> Deployment {
    Deployment::from_chat(&listed("openai/up", "http://127.0.0.1:9/v1"))
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn chat() -> OperationRequest {
    OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("up"),
        request: ChatRequest::parse(br#"{"model":"g","messages":[]}"#).expect("request"),
        control: ControlOptions::default(),
    }))
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy");
    ProxiedClient::outbound(&Arc::new(policy)).expect("client")
}

#[rstest]
#[case::azure(Provider::Azure(Azure), "azure")]
#[case::azure_ai(Provider::AzureAi(AzureAi), "azure_ai")]
#[case::anthropic(Provider::Anthropic(Anthropic), "anthropic")]
#[case::bedrock(Provider::Bedrock(Bedrock), "bedrock")]
#[case::vertex_ai(Provider::VertexAi(VertexAi), "vertex_ai")]
#[case::gemini(Provider::Gemini(Gemini), "gemini")]
#[case::xai_mistral(Provider::XaiMistral(XaiMistral), "xai_mistral")]
#[case::embedding_rerank(Provider::EmbeddingRerank(EmbeddingRerank), "embedding_rerank")]
#[case::compatible(Provider::Compatible(Compatible), "compatible")]
#[case::legacy(Provider::Legacy(Legacy), "legacy")]
#[case::self_hosted(Provider::SelfHosted(SelfHosted), "self_hosted")]
#[case::subscription(Provider::Subscription(Subscription), "subscription")]
#[case::media(Provider::MediaProviders(MediaProviders), "media_providers")]
#[case::speech(Provider::SpeechProviders(SpeechProviders), "speech_providers")]
#[tokio::test]
async fn provider_framework_tests_behavior(#[case] provider: Provider, #[case] name: &str) {
    let request = chat();
    let deployment = deployment();
    let refused = provider
        .prepare(&request, &deployment, &catalog())
        .expect_err("a stub provider never prepares a request");
    assert_eq!(
        refused.to_string(),
        format!("betterlite has no executable adapter for the {name} provider")
    );

    let prepared = Provider::OpenAi(OpenAi)
        .prepare(&request, &deployment, &catalog())
        .expect("the openai adapter prepares chat");
    let client = client();
    let sent = match provider.execute(&client, &prepared, &deployment).await {
        Ok(_answer) => panic!("a stub provider never sends a request"),
        Err(error) => error,
    };
    assert_eq!(
        sent.to_string(),
        format!("betterlite has no executable adapter for the {name} provider")
    );

    let upstream = Upstream::Buffered(UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: b"{}".to_vec(),
    });
    let decoded = match provider.decode(&prepared, upstream) {
        Ok(_answer) => panic!("a stub provider never decodes an answer"),
        Err(error) => error,
    };
    assert!(matches!(decoded, OperationError::UnavailableAdapter { .. }));
}

mod foreign_operation;
mod openai;
mod resolution;

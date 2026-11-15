#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tracing::Level;
use tracing::subscriber::with_default;

use super::catalog::{CompletionCatalog, CompletionRoute, route_for};
use super::failure::wrapped_failure;
use super::http::HttpCompletionConstructor;
use super::log::TracingCompletionLog;
use super::transport_request;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use crate::text_completion_request::CompletionRequest;
use crate::text_completion_transport::error::{ErrorSource, ErrorStatus, TextCompletionError};
use crate::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, ResponseKind, SecretHeaders,
};
use crate::text_completion_transport::sdk::{
    ClientConstructor as _, ClientKind, HttpClientSource, SdkArguments, TextCompletionSdk as _,
};

fn deployment(model: &str) -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("g"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(model.to_owned()),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
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
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

fn catalog() -> CompletionCatalog {
    CompletionCatalog {
        chat_models: BTreeSet::from([String::from("gpt-4o")]),
        text_models: BTreeSet::from([
            String::from("gpt-3.5-turbo-instruct"),
            String::from("text-davinci-003"),
        ]),
    }
}

fn completion(prompt: &str) -> CompletionRequest {
    let body = format!(r#"{{"model":"gpt-3.5-turbo-instruct","prompt":{prompt}}}"#);
    CompletionRequest::parse(body.as_bytes()).expect("completion request")
}

// a deployment that inherited litellm_settings.organization or OPENAI_ORGANIZATION
fn deployment_with_organization(model: &str) -> ChatDeployment {
    let mut deployment = deployment(model);
    deployment.organization = Some(String::from("org-1"));
    deployment
}

// a provider whose identity resolution refused it, the way a text-completion or azure prefix resolves today
fn unsupported(model: &str) -> ChatDeployment {
    let mut deployment = deployment("openai/gpt-4o");
    model.clone_into(&mut deployment.litellm_model);
    deployment.provider = Provider::Unsupported {
        model: model.to_owned(),
    };
    deployment
}

fn provider_error(status: u16) -> TextCompletionError {
    TextCompletionError {
        status: ErrorStatus::Code(status),
        message: String::from("the provider refused the call"),
        headers: None,
        source: Box::new(ErrorSource::Raised),
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    StringPrompt,
    StringsPrompt,
    TokensPrompt,
    TokenListsPrompt,
    EmptyStringPrompt,
    EmptyListRejected,
    MixedPromptRejected,
    ChatCatalogSelection,
    BundledCatalog,
    ProviderErrorFacts,
    ConfiguredDestination,
    UnsupportedProvider,
}

#[rstest]
#[case::string_prompt(Scenario::StringPrompt)]
#[case::strings_prompt(Scenario::StringsPrompt)]
#[case::tokens_prompt(Scenario::TokensPrompt)]
#[case::token_lists_prompt(Scenario::TokenListsPrompt)]
#[case::empty_string_prompt(Scenario::EmptyStringPrompt)]
#[case::empty_list_rejected(Scenario::EmptyListRejected)]
#[case::mixed_prompt_rejected(Scenario::MixedPromptRejected)]
#[case::chat_catalog_selection(Scenario::ChatCatalogSelection)]
#[case::bundled_catalog(Scenario::BundledCatalog)]
#[case::provider_error_facts(Scenario::ProviderErrorFacts)]
#[case::configured_destination(Scenario::ConfiguredDestination)]
#[case::unsupported_provider(Scenario::UnsupportedProvider)]
fn text_completions_text_completion_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StringPrompt => {
            let request = completion(r#""hello""#);
            let messages = request.prompt.messages().expect("messages");
            assert_eq!(messages.len(), 1);
        }
        Scenario::StringsPrompt => {
            let request = completion(r#"["a","b"]"#);
            let messages = request.prompt.messages().expect("messages");
            assert_eq!(messages.len(), 2);
        }
        Scenario::TokensPrompt => {
            let request = completion("[1,2]");
            let messages = request.prompt.messages().expect("messages");
            assert_eq!(messages.len(), 1);
        }
        Scenario::TokenListsPrompt => {
            let request = completion("[[1,2],[3]]");
            let messages = request.prompt.messages().expect("messages");
            assert_eq!(messages.len(), 1);
        }
        Scenario::EmptyStringPrompt => {
            let request = completion(r#""""#);
            assert!(request.prompt.messages().is_some());
        }
        Scenario::EmptyListRejected => {
            let request = completion("[]");
            assert!(request.prompt.messages().is_none());
        }
        Scenario::MixedPromptRejected => {
            let request = completion(r#"["a",1]"#);
            assert!(request.prompt.messages().is_none());
        }
        Scenario::ChatCatalogSelection => {
            assert_eq!(
                route_for(&deployment("openai/gpt-4o"), &catalog()),
                CompletionRoute::Chat {
                    model: String::from("gpt-4o")
                }
            );
            assert_eq!(
                route_for(&deployment("openai/gpt-3.5-turbo-instruct"), &catalog()),
                CompletionRoute::Text {
                    model: String::from("gpt-3.5-turbo-instruct")
                }
            );
            assert_eq!(
                route_for(&unsupported("text-completion-openai/gpt-4o"), &catalog()),
                CompletionRoute::Text {
                    model: String::from("gpt-4o")
                }
            );
            assert_eq!(
                route_for(&unsupported("gpt-3.5-turbo-instruct"), &catalog()),
                CompletionRoute::Text {
                    model: String::from("gpt-3.5-turbo-instruct")
                }
            );
            // an unprefixed name still needs the text catalog, so a completion model newer than the bundle refuses
            assert_eq!(
                route_for(&unsupported("some-new-model"), &catalog()),
                CompletionRoute::Unsupported {
                    provider: String::from("some-new-model")
                }
            );
        }
        Scenario::BundledCatalog => {
            let bundled = CompletionCatalog::bundled();
            assert!(bundled.chat_models.contains("gpt-4o"));
            assert!(bundled.text_models.contains("gpt-3.5-turbo-instruct"));
            assert!(!bundled.text_models.contains("gpt-4o"));
        }
        Scenario::ProviderErrorFacts => {
            let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
            let failure = wrapped_failure(&provider_error(429), &redactor);
            assert_eq!(failure.status, 429);
            assert_eq!(failure.error_type.as_deref(), Some("throttling_error"));
            assert_eq!(failure.param, None);
            assert!(!failure.message.is_empty());
            let refused = wrapped_failure(&provider_error(400), &redactor);
            assert_eq!(refused.status, 400);
            assert_eq!(refused.error_type, None);
        }
        Scenario::ConfiguredDestination => {
            let request = completion(r#""hi""#);
            let deployment = deployment("openai/gpt-3.5-turbo-instruct");
            let built = transport_request(&deployment, &request, "gpt-3.5-turbo-instruct")
                .expect("transport request");
            assert_eq!(built.api_base.as_deref(), Some("http://127.0.0.1:9/v1"));
            assert!(built.timeout > 0.0);
            assert!(built.acompletion);
        }
        Scenario::UnsupportedProvider => {
            assert!(matches!(
                route_for(&unsupported("azure/gpt-35-turbo"), &catalog()),
                CompletionRoute::Unsupported { .. }
            ));
        }
    }
}

#[test]
fn an_unmapped_prompt_fails_before_any_provider_call() {
    let request = completion("[]");
    let failure = transport_request(&deployment("openai/gpt-3.5-turbo-instruct"), &request, "m")
        .err()
        .expect("unmapped prompt");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.error_type, None);

    let unreadable = completion(r#"[["a"]]"#);
    let failure = transport_request(
        &deployment("openai/gpt-3.5-turbo-instruct"),
        &unreadable,
        "m",
    )
    .err()
    .expect("unreadable prompt");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.error_type.as_deref(), Some("internal_server_error"));
}

#[test]
fn the_completion_call_takes_the_body_timeout_and_never_the_configured_organization() {
    let deployment = deployment_with_organization("openai/gpt-3.5-turbo-instruct");
    let request = completion(r#""hi""#);
    let built = transport_request(&deployment, &request, "gpt-3.5-turbo-instruct")
        .expect("transport request");
    assert!(built.organization.is_none());
    assert!((built.timeout - deployment.timeout_seconds).abs() < f64::EPSILON);

    let timed = CompletionRequest::parse(
        br#"{"model":"gpt-3.5-turbo-instruct","prompt":"hi","timeout":2.5}"#,
    )
    .expect("completion request");
    let built = transport_request(&deployment, &timed, "gpt-3.5-turbo-instruct")
        .expect("transport request");
    assert!((built.timeout - 2.5).abs() < f64::EPSILON);
}

#[test]
fn the_route_logger_records_counts_and_never_the_prompt() {
    let log = NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("log handle")))
        .finish();
    let defaults = with_default(subscriber, || {
        let mut logger = TracingCompletionLog;
        logger
            .pre_call(PreCallRecord {
                input: vec![json!({"role": "user", "content": "secret prompt"})],
                api_key: None,
                headers: SecretHeaders::default(),
                api_base: Some(String::from("http://upstream.test/v1")),
                complete_input_dict: Map::new(),
            })
            .expect("pre-call");
        logger.post_call(PostCallRecord {
            api_key: None,
            original_response: Value::Null,
            kind: ResponseKind::SdkObject,
            headers: SecretHeaders::default(),
            api_base: None,
        });
        logger.stream_defaults()
    });
    let text = fs::read_to_string(log.path()).expect("log text");
    assert!(text.contains("text completion pre-call"), "{text}");
    assert!(text.contains("messages=1"), "{text}");
    assert!(text.contains("http://upstream.test/v1"), "{text}");
    assert!(text.contains("text completion post-call"), "{text}");
    assert!(text.contains("SdkObject"), "{text}");
    assert!(!text.contains("secret prompt"), "{text}");
    assert!(defaults.stream_options.is_none());
    assert!(!defaults.merge_reasoning_in_choices);
    assert!(!defaults.is_function_call);
    assert!(defaults.base_hidden_params.is_empty());
}

// the request head the completion client sent, read byte by byte so no header line is cut short
async fn read_completion_head(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let Ok(byte) = stream.read_u8().await else {
            break;
        };
        received.push(byte);
    }
    String::from_utf8(received).unwrap_or_default()
}

// a provider that takes the whole request before it answers, so a buffered client never sees a reset while writing
async fn serve_text_completion(listener: TcpListener) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(5), listener.accept()).await else {
        return String::new();
    };
    let head = read_completion_head(&mut stream).await;
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    let mut body = vec![0_u8; length];
    let _filled = stream.read_exact(&mut body).await;
    let answer = "{}";
    let response = format!(
        "HTTP/1.1 200 X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    let _written = stream.write_all(response.as_bytes()).await;
    head
}

#[tokio::test]
async fn the_http_sdk_sends_the_credential_and_never_an_organization() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("client");
    let arguments = SdkArguments {
        api_key: None,
        base_url: Some(format!("http://{address}/v1")),
        timeout: 5.0,
        max_retries: None,
        organization: None,
        http_client: HttpClientSource::AsyncHttpClient,
    };
    let constructor = HttpCompletionConstructor {
        client,
        // deployment.credential() hands the client a complete header value, bearer prefix included
        authorization: Some(SecretString::from("Bearer sk-deployment")),
        facts: Arc::new(Mutex::new(None)),
    };
    let sdk = constructor
        .construct(&arguments, ClientKind::Async)
        .expect("sdk")
        .expect("a credential builds the sdk");
    assert_eq!(sdk.kind(), ClientKind::Async);
    let mut data = Map::new();
    data.insert(String::from("model"), Value::from("gpt-3.5-turbo-instruct"));
    data.insert(String::from("prompt"), Value::from("hi"));
    let (answer, head) = tokio::join!(sdk.create(&data), serve_text_completion(listener));
    assert!(answer.is_ok(), "{:?}", answer.err());
    let sent = head.to_ascii_lowercase();
    assert!(sent.starts_with("post /v1/completions "), "{head}");
    assert!(
        sent.contains("authorization: bearer sk-deployment"),
        "{head}"
    );
    assert!(!sent.contains("openai-organization"), "{head}");
}

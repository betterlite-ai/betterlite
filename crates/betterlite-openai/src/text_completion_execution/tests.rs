#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;

use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::with_default;

use super::catalog::{CompletionCatalog, CompletionRoute, route_for};
use super::failure::wrapped_failure;
use super::log::TracingCompletionLog;
use super::transport_request;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use crate::text_completion_request::CompletionRequest;
use crate::text_completion_transport::error::{ErrorSource, ErrorStatus, TextCompletionError};
use crate::text_completion_transport::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, ResponseKind, SecretHeaders,
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
            let request = completion(r#"[1,2]"#);
            let messages = request.prompt.messages().expect("messages");
            assert_eq!(messages.len(), 1);
        }
        Scenario::TokenListsPrompt => {
            let request = completion(r#"[[1,2],[3]]"#);
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
        }
        Scenario::ProviderErrorFacts => {
            let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
            let failure = wrapped_failure(&provider_error(429), &redactor);
            assert_eq!(failure.status, 429);
            assert_eq!(failure.error_type, "rate_limit_error");
            assert_eq!(failure.param, None);
            assert!(!failure.message.is_empty());
            let refused = wrapped_failure(&provider_error(400), &redactor);
            assert_eq!(refused.error_type, "invalid_request_error");
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
        .expect_err("unmapped prompt");
    assert_eq!(failure.status, 500);
    assert_eq!(failure.error_type, "internal_server_error");
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

#![cfg(test)]

mod base_options;
mod guardrail;
mod llama;
mod metadata;
mod model_url;
mod prompt;
mod provider_selection;
mod signing;
mod support;

use std::collections::BTreeMap;
use std::future::ready;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{
    ChatTemplate, Hub, HubClient, TokenizerConfig, TokenizerDocument,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::environment::error_class;
use crate::invoke_primitives::guardrail::{GuardrailError, guardrail_headers};
use crate::invoke_primitives::prompt::{
    ConversionError, PromptConversion, convert_messages_to_prompt,
};
use crate::invoke_primitives::signing::{SignRequest, sign_request};
use crate::invoke_primitives::url::{CompleteUrl, InvokeEnvironment, UrlError, invoke_model_url};
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;

use support::{clock, credentials, epoch, messages, params, request_data, runtime};

const NOW: i64 = 1_767_225_600;
const ENDPOINT: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude/invoke";
const LLAMA3_MODEL: &str = "meta-llama/Meta-Llama-3-8B-Instruct";

#[derive(Debug, Clone, Copy)]
enum Scenario {
    BaseOptions,
    ProviderFamilySelection,
    InvokeModelUrl,
    GuardrailHeaders,
    GuardrailMetadataMerge,
    SigningForwarding,
    PromptTemplateSelection,
    LlamaLocalTemplate,
}

#[rstest]
#[case::base_options(Scenario::BaseOptions)]
#[case::provider_family_selection(Scenario::ProviderFamilySelection)]
#[case::invoke_model_url(Scenario::InvokeModelUrl)]
#[case::guardrail_headers(Scenario::GuardrailHeaders)]
#[case::guardrail_metadata_merge(Scenario::GuardrailMetadataMerge)]
#[case::signing_forwarding(Scenario::SigningForwarding)]
#[case::prompt_template_selection(Scenario::PromptTemplateSelection)]
#[case::llama_local_template(Scenario::LlamaLocalTemplate)]
fn invoke_primitives_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BaseOptions => base_options::base_options(),
        Scenario::ProviderFamilySelection => provider_selection::provider_family_selection(),
        Scenario::InvokeModelUrl => model_url::invoke_model_url_case(),
        Scenario::GuardrailHeaders => guardrail::guardrail_headers_case(),
        Scenario::GuardrailMetadataMerge => metadata::guardrail_metadata_merge(),
        Scenario::SigningForwarding => signing::signing_forwarding(),
        Scenario::PromptTemplateSelection => prompt::prompt_template_selection(),
        Scenario::LlamaLocalTemplate => llama::llama_local_template(),
    }
}

fn signing_request<'text>(
    headers: &'text [(String, Option<String>)],
    optional_params: &'text Params,
    data: &'text IndexMap<String, Json>,
    api_key: Option<&'text SecretString>,
    now: SystemTime,
) -> SignRequest<'text> {
    SignRequest {
        headers,
        optional_params,
        request_data: data,
        api_base: ENDPOINT,
        api_key,
        configured_bearer: None,
        model: Some("anthropic.claude-v2"),
        stream: None,
        fake_stream: None,
        region_inputs: RegionInputs::default(),
        now,
    }
}

fn authorization(signed: &AwsSignedRequest) -> &str {
    signed
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str())
        .unwrap()
}

fn sign_with_credentials(fixture: Credentials) -> AwsSignedRequest {
    let optional = params("{}");
    let data = request_data(r#"{"inputText":"hello"}"#);
    let headers = vec![(
        "content-type".to_owned(),
        Some("application/json".to_owned()),
    )];
    runtime()
        .block_on(sign_request(
            signing_request(&headers, &optional, &data, None, epoch(NOW)),
            move |_settings, _region| ready(Ok::<Credentials, CredentialError>(fixture)),
        ))
        .unwrap()
}

fn sign_with_bearer(key: &SecretString) -> AwsSignedRequest {
    let optional = params("{}");
    let data = request_data(r#"{"inputText":"hello"}"#);
    let headers = vec![(
        "content-type".to_owned(),
        Some("application/json".to_owned()),
    )];
    runtime()
        .block_on(sign_request(
            signing_request(&headers, &optional, &data, Some(key), epoch(NOW)),
            |_settings, _region| {
                ready(Err::<Credentials, CredentialError>(
                    CredentialError::MissingCredentials,
                ))
            },
        ))
        .unwrap()
}

fn render_with_template(template: &str) -> String {
    let prompts = PromptFactory {
        cache: Mutex::new(BTreeMap::from([(
            LLAMA3_MODEL.to_owned(),
            TokenizerConfig::Success(Some(TokenizerDocument {
                chat_template: ChatTemplate::Text(template.to_owned()),
                bos_token: None,
                eos_token: None,
            })),
        )])),
        hub: Hub::Unreachable,
    };
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    runtime()
        .block_on(convert_messages_to_prompt(PromptConversion {
            model: "meta.llama3-8b-instruct-v1:0",
            messages: &exchange,
            provider: Some(BedrockInvokeProvider::Meta),
            custom_prompt_dict: &BTreeMap::new(),
            prompts: &prompts,
            now: clock(),
        }))
        .unwrap()
        .prompt
}

#[tokio::test]
async fn invoke_primitives_never_downloads_templates() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let prompts = PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Http(HubClient {
            base_url: format!("http://127.0.0.1:{port}"),
            client: ProxiedClient::outbound(&Arc::new(policy)).unwrap(),
        }),
    };
    let conversation = messages(r#"[{"role":"user","content":"hi"}]"#);
    let error = convert_messages_to_prompt(PromptConversion {
        model: "meta.llama3-8b-instruct-v1:0",
        messages: &conversation,
        provider: Some(BedrockInvokeProvider::Meta),
        custom_prompt_dict: &BTreeMap::new(),
        prompts: &prompts,
        now: clock(),
    })
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "No chat template found");
    assert!(matches!(
        error,
        ConversionError::Factory(FactoryError::NoChatTemplate)
    ));
    // the HTTP hub was never asked for the template
    match listener.accept() {
        Ok(_stream) => panic!("the local template lookup reached the network"),
        Err(failure) => assert_eq!(failure.kind(), ErrorKind::WouldBlock),
    }
}

#[test]
fn invoke_primitives_credential_identity_isolation() {
    signing_identities();
    bearer_identities();
    prompt_caches();
}

fn signing_identities() {
    let first = sign_with_credentials(credentials("FIRSTKEY", "first-secret"));
    let second = sign_with_credentials(credentials("SECONDKEY", "second-secret"));
    assert_ne!(authorization(&first), authorization(&second));
    assert!(authorization(&first).contains("FIRSTKEY"));
    assert!(!authorization(&first).contains("SECONDKEY"));
    assert!(
        first
            .headers
            .iter()
            .all(|(_, value)| !value.contains("second-secret"))
    );
}

fn bearer_identities() {
    let first_key = SecretString::from("first-api-key".to_owned());
    let second_key = SecretString::from("second-api-key".to_owned());
    let first = sign_with_bearer(&first_key);
    let second = sign_with_bearer(&second_key);
    assert_eq!(authorization(&first), "Bearer first-api-key");
    assert_eq!(authorization(&second), "Bearer second-api-key");
    assert_ne!(first.headers, second.headers);
}

fn prompt_caches() {
    assert_eq!(
        render_with_template("first {{ messages[0]['content'] }}"),
        "first hi"
    );
    assert_eq!(
        render_with_template("second {{ messages[0]['content'] }}"),
        "second hi"
    );
}

#[test]
fn invoke_primitives_secret_safe_failure() {
    signing_failure();
    credential_failure();
    url_failure();
    debug_output();
}

fn signing_failure() {
    let secret = "fixture-secret-access-key";
    let optional = params(
        r#"{"aws_access_key_id":"AKID","aws_secret_access_key":"fixture-secret-access-key","aws_region_name":"US-East-1"}"#,
    );
    let data = request_data("{}");
    let headers: Vec<(String, Option<String>)> = Vec::new();
    let failure = runtime()
        .block_on(sign_request(
            signing_request(&headers, &optional, &data, None, epoch(NOW)),
            |_settings, _region| {
                ready(Ok::<Credentials, CredentialError>(credentials(
                    "AKID", secret,
                )))
            },
        ))
        .unwrap_err();
    assert!(!format!("{failure} {failure:?}").contains(secret));
}

fn credential_failure() {
    let secret = "fixture-secret-access-key";
    let optional = params("{}");
    let data = request_data("{}");
    let headers: Vec<(String, Option<String>)> = Vec::new();
    let failure = runtime()
        .block_on(sign_request(
            signing_request(&headers, &optional, &data, None, epoch(NOW)),
            |_settings, _region| {
                ready(Err::<Credentials, CredentialError>(
                    CredentialError::MissingCredentials,
                ))
            },
        ))
        .unwrap_err();
    assert!(!format!("{failure} {failure:?}").contains(secret));
}

fn url_failure() {
    let secret = "fixture-secret-access-key";
    let optional =
        params(r#"{"aws_secret_access_key":"fixture-secret-access-key","aws_region_name":"BAD"}"#);
    let failure = invoke_model_url(CompleteUrl {
        api_base: None,
        model: "anthropic.claude-v2",
        optional_params: &optional,
        stream: None,
        environment: InvokeEnvironment::default(),
    })
    .unwrap_err();
    assert!(matches!(failure, UrlError::Endpoint(_)));
    assert!(!format!("{failure} {failure:?}").contains(secret));
}

fn debug_output() {
    let secret = "fixture-secret-access-key";
    let bearer = SecretString::from(secret.to_owned());
    let signed = sign_with_bearer(&bearer);
    assert!(!format!("{signed:?}").contains(secret));
    // B682's error Debug redacts the supplied headers
    let header_error = error_class(
        "boom".to_owned(),
        400,
        vec![("authorization".to_owned(), secret.to_owned())],
    )
    .unwrap();
    assert!(!format!("{header_error:?}").contains(secret));
    // the owned guardrail failure never names a credential
    let guardrail = guardrail_headers(
        &RawValue::from_string(r#"{"guardrailVersion":"1"}"#.to_owned()).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(
        guardrail,
        GuardrailError::MissingIdentifier { .. }
    ));
    assert!(!format!("{guardrail} {guardrail:?}").contains(secret));
}

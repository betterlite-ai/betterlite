#![cfg(test)]

mod send;

use super::fixtures::{Case, execute};
use rstest::rstest;
use serde::Deserialize;
use std::{fs, path::Path};
use tokio::runtime::Builder;

#[derive(Deserialize)]
struct Calls {
    call: Call,
}
#[derive(Deserialize)]
struct Call {
    anthropic_provider: Case,
}

#[rstest]
#[case::anthropic_chat_dispatch("anthropic-chat-dispatch")]
#[case::anthropic_legacy_dispatch("anthropic-legacy-dispatch")]
#[case::anthropic_count_dispatch("anthropic-count-dispatch")]
#[case::anthropic_batch_file_dispatch("anthropic-batch-file-dispatch")]
#[case::messages_default_base("messages-default-base")]
#[case::legacy_default_base("legacy-default-base")]
#[case::url_suffix_enabled("url-suffix-enabled")]
#[case::url_suffix_disabled("url-suffix-disabled")]
#[case::provider_config_selection("provider-config-selection")]
#[case::config_url_selector("config-url-selector")]
#[test]
fn anthropic_dispatch_cases(#[case] name: &str) {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle/cases/helpers/providers/anthropic-provider")
        .join(format!("{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: Calls = serde_saphyr::from_str(&source).unwrap();
    let actual = runtime
        .block_on(execute(&case.call.anthropic_provider))
        .unwrap();
    assert_eq!(actual, case.call.anthropic_provider.expected);
}

#[test]
fn anthropic_provider_credential_identity_isolation() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let mut case = single_chat_case();
    let mut second = case.call.anthropic_provider.inputs[0].clone();
    second.api_key = Some(secrecy::SecretString::from("sk-second-identity"));
    second.expected_key.clone_from(&second.api_key);
    second
        .headers
        .insert("x-oracle".to_owned(), secrecy::SecretString::from("second"));
    case.call.anthropic_provider.inputs.push(second);
    let first = runtime
        .block_on(execute(&case.call.anthropic_provider))
        .unwrap();
    let repeat = runtime
        .block_on(execute(&case.call.anthropic_provider))
        .unwrap();
    assert_eq!(first, repeat);
    assert_ne!(first[0], first[1]);
    let encoded = serde_json::to_string(&first).unwrap();
    assert_eq!(encoded.matches("\"auth_matches\":true").count(), 2);
    assert!(!encoded.contains("sk-second-identity"));
    assert!(!encoded.contains("sk-oracle-explicit"));
}

#[test]
fn anthropic_provider_secret_safe_failure() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let mut case = single_chat_case();
    case.call.anthropic_provider.inputs[0].failure = Some(503);
    let input = &mut case.call.anthropic_provider.inputs[0];
    input.headers.insert(
        "Authorization".to_owned(),
        secrecy::SecretString::from("Bearer sk-header-auth"),
    );
    input.headers.insert(
        "x-api-key".to_owned(),
        secrecy::SecretString::from("sk-header-key"),
    );
    input.environment.api_key = Some(secrecy::SecretString::from("sk-environment-key"));
    let call = super::completion::CompletionCall::from(&*input);
    let shown = format!("{call:?} {:?}", input.environment);
    assert!(!shown.contains("sk-header-auth"));
    assert!(!shown.contains("sk-header-key"));
    assert!(!shown.contains("sk-environment-key"));
    let globals = super::base::Globals {
        api_key: input.api_key.clone(),
        api_base: None,
        anthropic_key: None,
    };
    assert!(!format!("{globals:?}").contains("sk-oracle-explicit"));
    let result = runtime
        .block_on(execute(&case.call.anthropic_provider))
        .unwrap();
    let shown_result = format!("{result:?}");
    assert!(shown_result.contains("503"));
    assert!(!shown_result.contains("sk-oracle-explicit"));
    assert!(!shown_result.contains("sk-header-auth"));
    assert!(!shown_result.contains("sk-header-key"));
    assert!(!shown_result.contains("sk-environment-key"));
    assert!(!result[0].post_call);
}

fn single_chat_case() -> Calls {
    let source = include_str!(
        "../../../../tools/oracle/cases/helpers/providers/anthropic-provider/anthropic-chat-dispatch.yaml"
    );
    let mut case: Calls = serde_saphyr::from_str(source).unwrap();
    case.call.anthropic_provider.inputs.truncate(1);
    case.call.anthropic_provider.expected.truncate(1);
    case
}

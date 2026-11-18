#![cfg(test)]

use super::fixtures::{Case, execute};
use rstest::rstest;
use serde::Deserialize;

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
#[tokio::test]
async fn anthropic_dispatch_cases(#[case] name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/oracle/cases/helpers/providers/anthropic-provider")
        .join(format!("{name}.yaml"));
    let source = std::fs::read_to_string(path).unwrap();
    let case: Calls = serde_saphyr::from_str(&source).unwrap();
    let actual = execute(&case.call.anthropic_provider).await.unwrap();
    assert_eq!(actual, case.call.anthropic_provider.expected);
}

#[tokio::test]
async fn anthropic_provider_credential_identity_isolation() {
    let source = include_str!(
        "../../../../tools/oracle/cases/helpers/providers/anthropic-provider/anthropic-chat-dispatch.yaml"
    );
    let mut case: Calls = serde_saphyr::from_str(source).unwrap();
    case.call.anthropic_provider.inputs.truncate(1);
    let mut second = case.call.anthropic_provider.inputs[0].clone();
    second.api_key = Some(secrecy::SecretString::from("sk-second-identity"));
    second.expected_key = second.api_key.clone();
    second
        .headers
        .insert("x-oracle".to_owned(), "second".to_owned());
    case.call.anthropic_provider.inputs.push(second);
    let first = execute(&case.call.anthropic_provider).await.unwrap();
    let repeat = execute(&case.call.anthropic_provider).await.unwrap();
    assert_eq!(first, repeat);
    assert_ne!(first[0], first[1]);
    let encoded = serde_json::to_string(&first).unwrap();
    assert_eq!(encoded.matches("\"auth_matches\":true").count(), 2);
    assert!(!encoded.contains("sk-second-identity"));
    assert!(!encoded.contains("sk-oracle-explicit"));
}

#[tokio::test]
async fn anthropic_provider_secret_safe_failure() {
    let source = include_str!(
        "../../../../tools/oracle/cases/helpers/providers/anthropic-provider/anthropic-chat-dispatch.yaml"
    );
    let mut case: Calls = serde_saphyr::from_str(source).unwrap();
    case.call.anthropic_provider.inputs.truncate(1);
    case.call.anthropic_provider.inputs[0].failure = Some(503);
    let globals = super::base::Globals {
        api_key: case.call.anthropic_provider.inputs[0].api_key.clone(),
        ..super::base::Globals::default()
    };
    assert!(!format!("{globals:?}").contains("sk-oracle-explicit"));
    let result = execute(&case.call.anthropic_provider).await.unwrap();
    let shown = format!("{result:?}");
    assert!(shown.contains("503"));
    assert!(!shown.contains("sk-oracle-explicit"));
    assert!(!result[0].post_call);
}

use rstest::rstest;

use super::super::config::IngestionSetupError;
use super::*;

#[test]
fn credential_stored_values_win_over_caller_values() {
    let harness = Harness::default()
        .with_credentials(r#"{"custom_llm_provider": "bedrock", "vector_store_id": "vs_stored"}"#);
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "custom_llm_provider": "openai",
                "vector_store_id": "vs_caller",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert_eq!(
        text_of(&state.vector_store_config, "custom_llm_provider"),
        "bedrock"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "vector_store_id"),
        "vs_stored"
    );
}

#[rstest]
#[case::api_base("api_base")]
#[case::aws_sts_endpoint("aws_sts_endpoint")]
#[case::aws_web_identity_token("aws_web_identity_token")]
#[test]
fn credential_keeps_only_the_endpoint_the_credential_defines(#[case] stored: &str) {
    let harness = Harness::default().with_credentials(&format!(r#"{{"{stored}": "kept"}}"#));
    let state = harness.state(
        serde_json::from_str(
            r#"{
                "vector_store": {
                    "api_base": "https://caller.example",
                    "aws_sts_endpoint": "https://sts.caller",
                    "aws_web_identity_token": "caller-token",
                    "ttl_days": 7,
                    "litellm_credential_name": "creds"
                }
            }"#,
        )
        .expect("the options parse"),
    );
    assert_eq!(
        state.vector_store_config.contains_key("api_base"),
        stored == "api_base"
    );
    assert_eq!(
        state.vector_store_config.contains_key("aws_sts_endpoint"),
        stored == "aws_sts_endpoint"
    );
    assert_eq!(
        state
            .vector_store_config
            .contains_key("aws_web_identity_token"),
        stored == "aws_web_identity_token"
    );
    assert_eq!(text_of(&state.vector_store_config, stored), "kept");
    assert!(state.vector_store_config.contains_key("ttl_days"));
}

#[test]
fn credential_removes_every_endpoint_field_a_credential_omits() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "aws_sts_endpoint": "https://sts.caller",
                "aws_web_identity_token": "caller-token",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(state.vector_store_config.contains_key("api_key"));
    assert!(!state.vector_store_config.contains_key("api_base"));
    assert!(!state.vector_store_config.contains_key("aws_sts_endpoint"));
    assert!(
        !state
            .vector_store_config
            .contains_key("aws_web_identity_token")
    );
}

#[test]
fn credential_keeps_endpoint_fields_the_credential_defines() {
    let harness = Harness::default().with_credentials(
        r#"{
            "api_base": "https://stored.example",
            "aws_sts_endpoint": "https://sts.stored",
            "aws_web_identity_token": "stored-token"
        }"#,
    );
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "aws_sts_endpoint": "https://sts.caller",
                "aws_web_identity_token": "caller-token",
                "litellm_credential_name": "creds"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert_eq!(
        text_of(&state.vector_store_config, "api_base"),
        "https://stored.example"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "aws_sts_endpoint"),
        "https://sts.stored"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "aws_web_identity_token"),
        "stored-token"
    );
}

#[test]
fn credential_absent_name_keeps_the_caller_config() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "vector_store_id": "vs_1"}}"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(!state.vector_store_config.contains_key("api_key"));
    assert!(state.vector_store_config.contains_key("api_base"));
    assert!(state.vector_store_config.contains_key("vector_store_id"));
}

#[test]
fn credential_null_name_keeps_the_caller_config() {
    let harness = Harness::default().with_credentials(r#"{"api_key": "sk-stored"}"#);
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "litellm_credential_name": null}}"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(!state.vector_store_config.contains_key("api_key"));
    assert!(state.vector_store_config.contains_key("api_base"));
}

#[test]
fn credential_unknown_name_keeps_the_caller_config() {
    let parsed = serde_json::from_str(
        r#"{"vector_store": {"api_base": "https://caller.example", "litellm_credential_name": "missing"}}"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert!(state.vector_store_config.contains_key("api_base"));
}

#[test]
fn credential_empty_values_skip_the_merge_and_removal() {
    let harness = Harness::default().with_credentials("{}");
    let parsed = serde_json::from_str(
        r#"{
            "vector_store": {
                "api_base": "https://caller.example",
                "vector_store_id": "vs_1",
                "litellm_credential_name": "empty"
            }
        }"#,
    )
    .expect("the options parse");
    let state = harness.state(parsed);
    assert!(state.vector_store_config.contains_key("api_base"));
    assert!(state.vector_store_config.contains_key("vector_store_id"));
}

#[test]
fn credential_name_must_be_a_string() {
    let parsed = serde_json::from_str(r#"{"vector_store": {"litellm_credential_name": 5}}"#)
        .expect("the options parse");
    let Err(error) = IngestionState::new(
        parsed,
        None,
        IngestionPorts {
            ocr: Arc::new(FakeOcr::default()),
            uploader: Arc::new(FakeFetch::default()),
            credentials: Arc::new(FakeCredentials::default()),
        },
    ) else {
        panic!("the name is rejected");
    };
    assert!(matches!(
        error,
        IngestionSetupError::InvalidField {
            field: "litellm_credential_name",
            expected: "a string"
        }
    ));
}

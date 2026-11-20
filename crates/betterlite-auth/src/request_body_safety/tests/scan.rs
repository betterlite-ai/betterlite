use crate::request_body_safety::SafetyError;
use crate::request_body_safety::banned::URL_DESTINATION_FIELDS;

use super::{Fixture, Grants, body, denied, denied_param, passes};

#[test]
fn banned_observability_field() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(&fixture, r#"{"langfuse_host": "https://evil.example"}"#),
        "langfuse_host"
    );
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"langsmith_base_url": "https://evil.example"}"#
        ),
        "langsmith_base_url"
    );
}

#[test]
fn pricing_field_banned() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(&fixture, r#"{"input_cost_per_token": 0.1}"#),
        "input_cost_per_token"
    );
    assert_eq!(
        denied_param(&fixture, r#"{"output_cost_per_image_1024": 0.04}"#),
        "output_cost_per_image_1024"
    );
}

#[test]
fn root_model_list_always_denied() {
    let mut fixture = Fixture::default();
    fixture.allow = true;
    fixture.grants = Grants::exact(&[("model", &["model_list"])]);
    assert!(matches!(
        denied(&fixture, r#"{"model_list": []}"#),
        SafetyError::ModelList
    ));
}

#[test]
fn null_banned_field_present() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(&fixture, r#"{"azure_ad_token": null}"#),
        "azure_ad_token"
    );
}

#[test]
fn opt_in_only_literal_true() {
    let body = r#"{"aws_role_name": "role"}"#;
    assert_eq!(denied_param(&Fixture::default(), body), "aws_role_name");
    let mut opted_in = Fixture::default();
    opted_in.allow = true;
    passes(&opted_in, body);
}

#[test]
fn one_grant_does_not_skip_next() {
    let mut fixture = Fixture::default();
    fixture.grants = Grants::exact(&[("model", &["aws_role_name"])]);
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"aws_role_name": "role", "aws_session_name": "session"}"#
        ),
        "aws_session_name"
    );
}

#[test]
fn first_banned_field_order() {
    let fixture = Fixture::default();
    // azure_ad_token comes after aws_role_name in the pinned catalog
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"azure_ad_token": "token", "aws_role_name": "role"}"#
        ),
        "aws_role_name"
    );
}

#[test]
fn nested_container_order() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"aws_role_name": "role", "litellm_embedding_config": {"aws_session_name": "s"}}"#
        ),
        "aws_role_name"
    );
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"extra_body": {"aws_session_name": "s"}, "litellm_embedding_config": {"azure_ad_token": "t"}}"#
        ),
        "azure_ad_token"
    );
    assert_eq!(
        denied_param(&fixture, r#"{"extra_body": {"aws_role_name": "role"}}"#),
        "aws_role_name"
    );
}

#[test]
fn nested_url_denied() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"litellm_embedding_config": {"api_base": "https://evil.example"}}"#
        ),
        "api_base"
    );
    // the container may arrive as a JSON string
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"extra_body": "{\"api_base\": \"https://evil.example\"}"}"#
        ),
        "api_base"
    );
}

#[test]
fn metadata_not_object() {
    let fixture = Fixture::default();
    passes(&fixture, r#"{"metadata": ["langfuse_host"]}"#);
    passes(&fixture, r#"{"metadata": "not json"}"#);
    passes(&fixture, r#"{"metadata": 7}"#);
    passes(&fixture, r#"{"litellm_metadata": true}"#);
    passes(&fixture, r#"{"litellm_embedding_config": "[]"}"#);
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"metadata": "{\"langfuse_host\": \"https://evil\"}"}"#
        ),
        "langfuse_host"
    );
}

#[test]
fn json_metadata_and_bracket_form() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"litellm_metadata": "{\"aws_role_name\": \"role\"}"}"#
        ),
        "aws_role_name"
    );
    assert_eq!(
        denied_param(&fixture, r#"{"metadata[langfuse_host]": "https://evil"}"#),
        "langfuse_host"
    );
    assert_eq!(
        denied_param(&fixture, r#"{"litellm_metadata[aws_role_name]": "role"}"#),
        "aws_role_name"
    );
    // only the extracted top level is scanned, so a nested path under an innocent name passes
    passes(
        &fixture,
        r#"{"metadata[team][langfuse_host]": "https://evil"}"#,
    );
    passes(&fixture, r#"{"metadata[team]": "value"}"#);
}

#[test]
fn one_level_only() {
    let fixture = Fixture::default();
    passes(
        &fixture,
        r#"{"extra_body": {"nested": {"api_base": "https://evil"}}}"#,
    );
    passes(
        &fixture,
        r#"{"metadata": {"nested": {"langfuse_host": "https://evil"}}}"#,
    );
}

#[test]
fn nested_litellm_params_metadata() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"litellm_params": {"metadata": {"langfuse_host": "x"}}}"#
        ),
        "langfuse_host"
    );
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"litellm_params": "{\"metadata\": {\"aws_role_name\": \"r\"}}"}"#
        ),
        "aws_role_name"
    );
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"litellm_params": {"metadata": "{\"azure_ad_token\": \"t\"}"}}"#
        ),
        "azure_ad_token"
    );
    passes(
        &fixture,
        r#"{"litellm_params": {"other": {"langfuse_host": "x"}}}"#,
    );
}

#[test]
fn url_bans_survive_credential_opt_in() {
    let mut fixture = Fixture::default();
    fixture.allow = true;
    for field in URL_DESTINATION_FIELDS {
        let text = format!(r#"{{"{field}": "value"}}"#);
        assert_eq!(denied_param(&fixture, &text), field);
    }
}

#[test]
fn model_exact_then_provider_prefix() {
    let mut fixture = Fixture::default();
    fixture.grants = Grants::exact(&[
        ("openai/gpt-4", &["aws_role_name"]),
        ("openai", &["aws_session_name"]),
    ]);

    let granted = r#"{"aws_role_name": "role"}"#;
    fixture
        .check_json(granted, "openai/gpt-4")
        .expect("the exact group grants the field");
    // the full name is unknown, so the provider prefix answers
    fixture
        .check_json(r#"{"aws_session_name": "s"}"#, "openai/unknown")
        .expect("the provider prefix grants the field");
    assert_eq!(
        denied_param_for(&fixture, "openai/unknown", r#"{"aws_role_name": "role"}"#),
        "aws_role_name"
    );
    assert_eq!(
        denied_param_for(
            &fixture,
            "notaprovider/unknown",
            r#"{"aws_session_name": "s"}"#
        ),
        "aws_session_name"
    );

    // a known group without an opt-in does not fall back to its provider prefix
    let mut unset = Fixture::default();
    unset.grants = Grants::exact(&[("openai", &["aws_session_name"])]);
    unset.grants.0.insert("openai/gpt-4".to_owned(), None);
    assert_eq!(
        denied_param_for(&unset, "openai/gpt-4", r#"{"aws_session_name": "s"}"#),
        "aws_session_name"
    );
}

fn denied_param_for(fixture: &Fixture, model: &str, text: &str) -> String {
    match fixture.check_json(text, model) {
        Err(SafetyError::Banned { param }) => param,
        other => panic!("{text} should be refused for {model}, got {other:?}"),
    }
}

#[test]
fn no_model_info_denies() {
    let fixture = Fixture::default();
    assert_eq!(
        denied_param(&fixture, r#"{"aws_role_name": "role"}"#),
        "aws_role_name"
    );
    assert!(matches!(
        fixture.check_json(r#"{"model": "gpt-4"}"#, "gpt-4"),
        Ok(())
    ));
}

#[test]
fn string_param_grant() {
    let mut fixture = Fixture::default();
    fixture.grants = Grants::exact(&[("model", &["aws_role_name"])]);
    let granted = body(r#"{"aws_role_name": "role"}"#);
    fixture
        .check(&granted, "model")
        .expect("the exact parameter is granted");
    let paired = body(r#"{"aws_role_name": "role", "aws_session_name": "session"}"#);
    assert!(matches!(
        fixture.check(&paired, "model"),
        Err(SafetyError::Banned { ref param }) if param == "aws_session_name"
    ));
}

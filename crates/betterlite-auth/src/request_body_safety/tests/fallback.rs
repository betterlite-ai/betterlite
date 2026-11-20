use rstest::rstest;

use crate::request_body_safety::SafetyError;

use super::{Fixture, Grants, denied, denied_param, passes};

// the value inside the list at `depth`; the leaf sits in the list at the chain's last depth
fn list_at(depth: usize, levels: usize, leaf: &str) -> String {
    if depth == levels {
        return format!("[{leaf}]");
    }
    format!(
        r#"[{{"model": "m", "fallbacks": {}}}]"#,
        list_at(depth + 1, levels, leaf)
    )
}

fn chained(levels: usize, leaf: &str) -> String {
    format!(r#"{{"fallbacks": {}}}"#, list_at(0, levels, leaf))
}

#[test]
fn fallback_url_denied() {
    let fixture = Fixture::default();
    assert!(matches!(
        denied(&fixture, r#"{"fallbacks": ["https://evil.example/x"]}"#),
        SafetyError::FallbackDestination
    ));
    assert!(matches!(
        denied(
            &fixture,
            r#"{"fallbacks": ["a.test,https://evil.example"]}"#
        ),
        SafetyError::FallbackDestination
    ));
    assert!(matches!(
        denied(
            &fixture,
            r#"{"context_window_fallbacks": ["HTTPS://EVIL.EXAMPLE"]}"#
        ),
        SafetyError::FallbackDestination
    ));
    passes(
        &fixture,
        r#"{"fallbacks": ["openai/gpt-4", "gpt-4", "a.test"]}"#,
    );
}

#[rstest]
#[case::fallbacks("fallbacks")]
#[case::context_window("context_window_fallbacks")]
#[case::content_policy("content_policy_fallbacks")]
fn fallback_three_fields_and_override(#[case] field: &str) {
    let fixture = Fixture::default();
    let text = format!(r#"{{"{field}": ["https://evil.example"]}}"#);
    assert!(matches!(
        denied(&fixture, &text),
        SafetyError::FallbackDestination
    ));
    let overridden =
        format!(r#"{{"router_settings_override": {{"{field}": ["https://evil.example"]}}}}"#);
    assert!(matches!(
        denied(&fixture, &overridden),
        SafetyError::FallbackDestination
    ));
    // a stringified override is not a mapping, so LiteLLM never reads its fields
    passes(
        &fixture,
        r#"{"router_settings_override": "{\"fallbacks\": [\"https://evil.example\"]}"}"#,
    );
    // the root fields are traversed before the override's
    assert_eq!(
        denied_param(
            &fixture,
            r#"{"fallbacks": [{"aws_session_name": "s"}], "router_settings_override": {"fallbacks": [{"aws_role_name": "r"}]}}"#
        ),
        "aws_session_name"
    );
}

#[test]
fn fallback_list_mapping_shapes() {
    let fixture = Fixture::default();
    // an all-lists mapping is traversed but never yielded, so its own keys are not scanned
    passes(&fixture, r#"{"fallbacks": [{"azure_ad_token": ["x"]}]}"#);
    assert_eq!(
        denied_param(&fixture, r#"{"fallbacks": [{"azure_ad_token": "x"}]}"#),
        "azure_ad_token"
    );
    // a mapping with a model recurses only through its three fallback fields
    passes(
        &fixture,
        r#"{"fallbacks": [{"model": "gpt-4", "unknown": ["https://evil.example"]}]}"#,
    );
    // without a model every value is traversed
    assert!(matches!(
        denied(
            &fixture,
            r#"{"fallbacks": [{"unknown": ["https://evil.example"]}]}"#
        ),
        SafetyError::FallbackDestination
    ));
    // list items themselves are never recursed, only strings and mappings are
    passes(&fixture, r#"{"fallbacks": [["https://evil.example"]]}"#);
}

#[test]
fn fallback_target_original_model_policy() {
    let mut fixture = Fixture::default();
    fixture.grants = Grants::exact(&[("granted-model", &["aws_role_name"])]);
    fixture
        .check_json(
            r#"{"fallbacks": [{"model": "other-model", "aws_role_name": "role"}]}"#,
            "granted-model",
        )
        .expect("the request model's grant covers the target");
    assert_eq!(
        match fixture.check_json(
            r#"{"fallbacks": [{"model": "other-model", "azure_ad_token": "t"}]}"#,
            "granted-model"
        ) {
            Err(SafetyError::Banned { param }) => param,
            other => panic!("the target should be refused, got {other:?}"),
        },
        "azure_ad_token"
    );
}

#[test]
fn fallback_depth_boundary_before_type() {
    let fixture = Fixture::default();
    // 2 * ROUTER_MAX_FALLBACKS is 10: the tenth level still validates, the eleventh does not
    passes(&fixture, &chained(10, r#""ok""#));
    assert!(matches!(
        denied(&fixture, &chained(11, r#""ok""#)),
        SafetyError::FallbackDepth
    ));
    // the depth check runs before the type check, so a scalar value recursed at that depth fails too
    assert!(matches!(
        denied(&fixture, &chained(10, r#"{"other": "scalar"}"#)),
        SafetyError::FallbackDepth
    ));
    passes(&fixture, &chained(9, r#"{"other": "scalar"}"#));

    let mut zero = Fixture::default();
    zero.max_fallbacks = 0;
    passes(&zero, r#"{"fallbacks": ["ok"]}"#);
    assert!(matches!(
        denied(&zero, &chained(1, r#""ok""#)),
        SafetyError::FallbackDepth
    ));

    // a negative limit fails even the absent root fields, which are validated at depth zero
    let mut negative = Fixture::default();
    negative.max_fallbacks = -1;
    assert!(matches!(
        denied(&negative, "{}"),
        SafetyError::FallbackDepth
    ));
}

#[test]
fn fallback_nonurl_model() {
    let fixture = Fixture::default();
    passes(&fixture, r#"{"fallbacks": [{"model": "gpt-4"}]}"#);
    passes(&fixture, r#"{"fallbacks": [{"model": 7}]}"#);
    assert!(matches!(
        denied(
            &fixture,
            r#"{"fallbacks": [{"model": "https://evil.example"}]}"#
        ),
        SafetyError::FallbackDestination
    ));
    assert!(matches!(
        denied(
            &fixture,
            r#"{"fallbacks": [{"model": "HTTPS://EVIL.EXAMPLE"}]}"#
        ),
        SafetyError::FallbackDestination
    ));
}

use std::collections::BTreeMap;
use std::sync::Mutex;

use rstest::rstest;

use super::fixtures::json;
use crate::database_reencryption::MigrationError;
use crate::database_reencryption::registry::registered_columns;
use crate::database_reencryption::traversal::{as_sql_parameter, replace_ciphertexts};
use crate::encryption_scan::MigrationJson;

fn rendered(value: &MigrationJson) -> String {
    serde_json::to_string(value).expect("the value serializes")
}

fn replace(
    value: &str,
    replacements: &[(&str, Option<&str>)],
    depth: i64,
    max_depth: i64,
) -> Result<(MigrationJson, usize), MigrationError> {
    let map: BTreeMap<&str, Option<&str>> = replacements.iter().copied().collect();
    let mut replacement = |text: &str| {
        Ok::<_, MigrationError>(match map.get(text) {
            Some(Some(replacement)) => Some((*replacement).to_owned()),
            Some(None) | None => None,
        })
    };
    replace_ciphertexts(json(value), &mut replacement, depth, max_depth)
}

#[test]
fn scalar_and_container_preservation() {
    let value = r#"{"a":null,"b":true,"c":3,"d":1.5,"e":["x",{"f":"y"}],"g":{}}"#;
    let (replaced, leaves) = replace(value, &[("x", Some("X")), ("y", Some("Y"))], 0, 100)
        .expect("the traversal succeeds");
    assert_eq!(leaves, 2);
    assert_eq!(
        rendered(&replaced),
        r#"{"a":null,"b":true,"c":3,"d":1.5,"e":["X",{"f":"Y"}],"g":{}}"#
    );
}

#[test]
fn dictionary_keys_not_replaced() {
    let (replaced, leaves) = replace(
        r#"{"cipher":"cipher"}"#,
        &[("cipher", Some("changed"))],
        0,
        100,
    )
    .expect("the traversal succeeds");
    assert_eq!(leaves, 1);
    assert_eq!(rendered(&replaced), r#"{"cipher":"changed"}"#);
}

#[test]
fn marker_removed_once_and_restored() {
    let (single, single_leaves) = replace(
        r#""litellm_enc::abc""#,
        &[("abc", Some("new")), ("litellm_enc::abc", Some("double"))],
        0,
        100,
    )
    .expect("the traversal succeeds");
    assert_eq!(single_leaves, 1);
    assert_eq!(rendered(&single), r#""litellm_enc::new""#);
    let (double, double_leaves) = replace(
        r#""litellm_enc::litellm_enc::abc""#,
        &[("litellm_enc::abc", Some("inner"))],
        0,
        100,
    )
    .expect("the traversal succeeds");
    assert_eq!(double_leaves, 1);
    assert_eq!(rendered(&double), r#""litellm_enc::inner""#);
    let (untouched, untouched_leaves) = replace(
        r#""litellm_enc::litellm_enc::abc""#,
        &[("abc", Some("inner"))],
        0,
        100,
    )
    .expect("the traversal succeeds");
    assert_eq!(untouched_leaves, 0);
    assert_eq!(rendered(&untouched), r#""litellm_enc::litellm_enc::abc""#);
}

#[test]
fn replacement_none_vs_empty_vs_identical() {
    let (replaced, leaves) = replace(
        r#"["absent","empty","same"]"#,
        &[("empty", Some("")), ("same", Some("same"))],
        0,
        100,
    )
    .expect("the traversal succeeds");
    assert_eq!(leaves, 2);
    assert_eq!(rendered(&replaced), r#"["absent","","same"]"#);
}

#[test]
fn json_string_not_recursively_parsed() {
    let inner = r#"{"a":"secret"}"#;
    let quoted = format!("\"{}\"", inner.replace('"', "\\\""));
    let (untouched, untouched_leaves) =
        replace(&quoted, &[("secret", Some("other"))], 0, 100).expect("the traversal succeeds");
    assert_eq!(untouched_leaves, 0);
    assert_eq!(rendered(&untouched), quoted);
    let (whole, whole_leaves) =
        replace(&quoted, &[(inner, Some("whole"))], 0, 100).expect("the traversal succeeds");
    assert_eq!(whole_leaves, 1);
    assert_eq!(rendered(&whole), "\"whole\"");
}

#[test]
fn depth_boundary_inclusive() {
    let (at_limit, at_limit_leaves) =
        replace(r#"["secret"]"#, &[("secret", Some("new"))], 0, 1).expect("the traversal");
    assert_eq!(at_limit_leaves, 1);
    assert_eq!(rendered(&at_limit), r#"["new"]"#);
    let (past_limit, past_limit_leaves) =
        replace(r#"["secret"]"#, &[("secret", Some("new"))], 0, 0).expect("the traversal");
    assert_eq!(past_limit_leaves, 0);
    assert_eq!(rendered(&past_limit), r#"["secret"]"#);
    let (root, root_leaves) =
        replace(r#""secret""#, &[("secret", Some("new"))], 0, 0).expect("the traversal");
    assert_eq!(root_leaves, 1);
    assert_eq!(rendered(&root), r#""new""#);
}

#[test]
fn zero_and_negative_depth_limits() {
    let (zero, zero_leaves) =
        replace(r#""secret""#, &[("secret", Some("new"))], 0, 0).expect("the traversal");
    assert_eq!(
        (rendered(&zero), zero_leaves),
        (String::from(r#""new""#), 1)
    );
    let (minus_one, minus_one_leaves) = replace(
        r#"["secret","other"]"#,
        &[("secret", Some("new")), ("other", Some("new"))],
        0,
        -1,
    )
    .expect("the traversal");
    assert_eq!(minus_one_leaves, 0);
    assert_eq!(rendered(&minus_one), r#"["secret","other"]"#);
    let (minus_five, minus_five_leaves) = replace(
        r#"["secret","other"]"#,
        &[("secret", Some("new")), ("other", Some("new"))],
        0,
        -5,
    )
    .expect("the traversal");
    assert_eq!(minus_five_leaves, 0);
    assert_eq!(rendered(&minus_five), r#"["secret","other"]"#);
}

#[test]
fn child_error_stops_traversal() {
    let calls = Mutex::new(Vec::new());
    let mut replacement = |text: &str| {
        calls.lock().expect("the calls lock").push(text.to_owned());
        if text == "fail" {
            return Err(MigrationError::NotText);
        }
        Ok(Some(String::from("ok")))
    };
    let result = replace_ciphertexts(
        json(r#"["first","fail","third"]"#),
        &mut replacement,
        0,
        100,
    );
    assert!(matches!(result, Err(MigrationError::NotText)));
    assert_eq!(
        calls.into_inner().expect("the calls lock"),
        vec!["first", "fail"]
    );
}

#[rstest]
#[case::json_object(r#"{"a":1}"#, true, r#"{"a":1}"#)]
#[case::json_string(r#""text""#, true, r#""text""#)]
#[case::json_number("1", true, "1")]
#[case::scalar_text(r#""text""#, false, "text")]
fn json_vs_scalar_sql_parameter(
    #[case] value: &str,
    #[case] is_json: bool,
    #[case] expected: &str,
) {
    let parameter = as_sql_parameter(&json(value), is_json).expect("the shape is a parameter");
    assert_eq!(parameter, expected);
}

#[test]
fn json_vs_scalar_sql_parameter_rejects_a_non_string_scalar() {
    let error = as_sql_parameter(&json("1"), false).expect_err("a number is not a scalar value");
    assert!(matches!(error, MigrationError::NotText), "{error:?}");
}

#[test]
fn registry_keeps_lite_llm_order() {
    let columns: Vec<(&str, &str)> = registered_columns()
        .map(|column| (column.table, column.column))
        .collect();
    assert_eq!(
        columns,
        vec![
            ("LiteLLM_ProxyModelTable", "litellm_params"),
            ("LiteLLM_CredentialsTable", "credential_values"),
            ("LiteLLM_Config", "param_value"),
            ("LiteLLM_SSOConfig", "sso_settings"),
            ("LiteLLM_CacheConfig", "cache_settings"),
            ("LiteLLM_ConfigOverrides", "config_value"),
            ("LiteLLM_MCPServerTable", "credentials"),
            ("LiteLLM_MCPServerTable", "static_headers"),
            ("LiteLLM_MCPServerTable", "env_vars"),
            ("LiteLLM_MCPServerTable", "env"),
            ("LiteLLM_MCPServerOAuthClient", "credentials"),
            ("LiteLLM_MCPUserCredentials", "credential_b64"),
            ("LiteLLM_MCPUserEnvVars", "values_b64"),
            ("LiteLLM_SSOIdentityAssertion", "assertion_b64"),
            ("LiteLLM_TeamTable", "metadata"),
            ("LiteLLM_VerificationToken", "metadata"),
            ("LiteLLM_UserTable", "metadata"),
            ("LiteLLM_DeletedTeamTable", "metadata"),
            ("LiteLLM_DeletedVerificationToken", "metadata"),
        ]
    );
}

#[test]
fn registry_marks_scalars_and_marked_rows() {
    let registered = registered_columns().collect::<Vec<_>>();
    assert_eq!(
        registered
            .iter()
            .filter(|column| !column.is_json)
            .map(|column| column.column)
            .collect::<Vec<_>>(),
        vec!["credential_b64", "values_b64", "assertion_b64"]
    );
    assert_eq!(
        registered
            .iter()
            .filter(|column| column.marked_only)
            .map(|column| column.column)
            .collect::<Vec<_>>(),
        vec!["metadata", "metadata", "metadata", "metadata", "metadata"]
    );
}

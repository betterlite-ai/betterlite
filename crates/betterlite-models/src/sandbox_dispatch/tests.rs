#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::{SandboxToolConfig, SandboxToolEntry, SandboxToolEntryError, sandbox_tool_entries};
use crate::decisions::Json;

fn value(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn parsed(source: &str) -> Vec<SandboxToolEntry> {
    sandbox_tool_entries(value(source)).unwrap()
}

fn error(source: &str) -> SandboxToolEntryError {
    sandbox_tool_entries(value(source)).unwrap_err()
}

#[test]
fn reads_the_name_provider_key_and_base_of_each_entry() {
    let entries = parsed(
        r#"[{"sandbox_tool_name":"tools","litellm_params":{"sandbox_provider":"e2b","api_key":"os.environ/E2B_API_KEY","api_base":"https://e2b.test"}}]"#,
    );
    assert_eq!(
        entries,
        vec![SandboxToolEntry::Configured(SandboxToolConfig {
            name: Some("tools".to_owned()),
            provider: Some("e2b".to_owned()),
            api_key: Some(SecretString::from("os.environ/E2B_API_KEY")),
            api_base: Some("https://e2b.test".to_owned()),
        })]
    );
}

#[rstest]
#[case::missing_params(r#"[{"sandbox_tool_name":"tools"}]"#)]
#[case::null_params(r#"[{"sandbox_tool_name":"tools","litellm_params":null}]"#)]
fn a_missing_or_null_params_mapping_leaves_the_provider_unset(#[case] source: &str) {
    let entries = parsed(source);
    assert_eq!(
        entries,
        vec![SandboxToolEntry::Configured(SandboxToolConfig {
            name: Some("tools".to_owned()),
            provider: None,
            api_key: None,
            api_base: None,
        })]
    );
}

#[test]
fn a_null_field_reads_as_absent() {
    let entries =
        parsed(r#"[{"sandbox_tool_name":null,"litellm_params":{"sandbox_provider":"e2b"}}]"#);
    let [SandboxToolEntry::Configured(config)] = entries.as_slice() else {
        panic!("one configured entry");
    };
    assert_eq!(config.name, None);
    assert_eq!(config.provider.as_deref(), Some("e2b"));
}

#[rstest]
#[case::text(r#"["not-a-tool"]"#)]
#[case::number("[5]")]
#[case::list("[[1,2]]")]
fn an_entry_that_is_not_a_mapping_keeps_its_place(#[case] source: &str) {
    assert_eq!(parsed(source), vec![SandboxToolEntry::NotAnObject]);
}

#[test]
fn a_non_mapping_keeps_the_configured_entries_in_order() {
    let entries = parsed(
        r#"[{"sandbox_tool_name":"first","litellm_params":{"sandbox_provider":"e2b"}},"skip",{"sandbox_tool_name":"last","litellm_params":{"sandbox_provider":"opensandbox"}}]"#,
    );
    assert_eq!(entries.len(), 3_usize);
    assert!(matches!(entries[1], SandboxToolEntry::NotAnObject));
}

#[rstest]
#[case::name(r#"[{"sandbox_tool_name":5}]"#, "sandbox_tools[0].sandbox_tool_name")]
#[case::provider(
    r#"[{"sandbox_tool_name":"t","litellm_params":{"sandbox_provider":true}}]"#,
    "sandbox_tools[0].litellm_params.sandbox_provider"
)]
#[case::api_key(
    r#"[{"sandbox_tool_name":"t","litellm_params":{"sandbox_provider":"e2b","api_key":["x"]}}]"#,
    "sandbox_tools[0].litellm_params.api_key"
)]
#[case::api_base(
    r#"[{"sandbox_tool_name":"t","litellm_params":{"sandbox_provider":"e2b","api_base":{}}}]"#,
    "sandbox_tools[0].litellm_params.api_base"
)]
fn a_field_of_the_wrong_type_is_an_error_naming_it(#[case] source: &str, #[case] path: &str) {
    assert_eq!(error(source).path(), path);
}

#[test]
fn litellm_params_that_is_not_a_mapping_is_an_error_naming_it() {
    let failure = error(r#"[{"sandbox_tool_name":"t","litellm_params":"e2b"}]"#);
    assert_eq!(failure.path(), "sandbox_tools[0].litellm_params");
}

#[test]
fn an_empty_list_reads_as_no_entries() {
    assert!(parsed("[]").is_empty());
}

#[test]
fn configs_that_differ_in_one_field_are_not_equal() {
    let base = SandboxToolConfig {
        name: Some("tools".to_owned()),
        provider: Some("e2b".to_owned()),
        api_key: Some(SecretString::from("os.environ/E2B_API_KEY")),
        api_base: Some("https://e2b.test".to_owned()),
    };
    let mut renamed = base.clone();
    renamed.name = Some("other".to_owned());
    assert_ne!(base, renamed);
    let mut unprovided = base.clone();
    unprovided.provider = None;
    assert_ne!(base, unprovided);
    let mut rekeyed = base.clone();
    rekeyed.api_key = Some(SecretString::from("os.environ/OTHER_API_KEY"));
    assert_ne!(base, rekeyed);
    let mut rebased = base.clone();
    rebased.api_base = None;
    assert_ne!(base, rebased);
    assert_eq!(base, base.clone());
}

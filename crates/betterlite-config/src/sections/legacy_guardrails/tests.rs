#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Raw, Settings};
use crate::loader::ConfigLoader;
use crate::sections::Section as _;
use crate::sections::guardrail_identity::enums::Role;
use crate::sections::guardrail_identity::item::{GuardrailItem, GuardrailValue};

fn settings(yaml: &str) -> Settings {
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    raw.convert(&BTreeMap::<String, String>::new()).unwrap()
}

fn only_item(yaml: &str) -> GuardrailItem {
    let parsed = settings(yaml);
    assert_eq!(parsed.items.len(), 1);
    parsed.items.into_iter().next().unwrap()
}

#[test]
fn legacy_config_absent_section_is_empty() {
    let empty_section = settings("litellm_settings: {}");
    assert!(empty_section.items.is_empty());
    assert!(empty_section.by_name.is_empty());
    let no_section = settings("model_list: []");
    assert!(no_section.items.is_empty());
}

#[test]
fn legacy_config_spec_defaults_follow_litellm() {
    let item = only_item(
        "litellm_settings:\n  guardrails:\n    - prompt_injection:\n        callbacks: [lakera_prompt_injection]\n",
    );
    assert_eq!(item.guardrail_name, "prompt_injection");
    assert_eq!(item.callbacks, ["lakera_prompt_injection"]);
    assert!(!item.default_on);
    assert_eq!(item.logging_only, None);
    assert_eq!(
        item.enabled_roles,
        Some(vec![Role::System, Role::Assistant, Role::User])
    );
    assert!(item.callback_args.is_empty());
}

#[test]
fn legacy_config_explicit_values_are_kept() {
    let item = only_item(
        "litellm_settings:\n  guardrails:\n    - pii:\n        callbacks: [presidio, other]\n        default_on: true\n        logging_only: false\n        enabled_roles: [user]\n        callback_args:\n          presidio:\n            language: en\n            score: 0.5\n",
    );
    assert_eq!(item.callbacks, ["presidio", "other"]);
    assert!(item.default_on);
    assert_eq!(item.logging_only, Some(false));
    assert_eq!(item.enabled_roles, Some(vec![Role::User]));
    let presidio = &item.callback_args["presidio"];
    assert_eq!(presidio["language"], GuardrailValue::Text("en".to_owned()));
    assert_eq!(presidio["score"], GuardrailValue::Decimal(0.5));
}

#[rstest]
#[case::null_roles("enabled_roles: null", None)]
#[case::empty_roles("enabled_roles: []", Some(vec![]))]
#[case::missing_roles("default_on: false", Some(vec![Role::System, Role::Assistant, Role::User]))]
fn legacy_config_enabled_roles(#[case] line: &str, #[case] expected: Option<Vec<Role>>) {
    let yaml = format!(
        "litellm_settings:\n  guardrails:\n    - g:\n        callbacks: [a]\n        {line}\n"
    );
    assert_eq!(only_item(&yaml).enabled_roles, expected);
}

#[test]
fn legacy_config_null_logging_only_and_callback_args_read_as_unset() {
    let item = only_item(
        "litellm_settings:\n  guardrails:\n    - g:\n        callbacks: [a]\n        logging_only: null\n        callback_args: null\n",
    );
    assert_eq!(item.logging_only, None);
    assert!(item.callback_args.is_empty());
}

#[rstest]
#[case::unknown_key("        callbacks: [a]\n        mode: pre_call\n")]
#[case::own_guardrail_name("        callbacks: [a]\n        guardrail_name: other\n")]
#[case::callbacks_required("        default_on: true\n")]
#[case::null_default_on("        callbacks: [a]\n        default_on: null\n")]
#[case::unknown_role("        callbacks: [a]\n        enabled_roles: [tool]\n")]
fn legacy_config_rejects_a_bad_spec(#[case] body: &str) {
    let yaml = format!("litellm_settings:\n  guardrails:\n    - g:\n{body}");
    serde_saphyr::from_str::<Raw>(&yaml).unwrap_err();
}

#[test]
fn legacy_config_by_name_keeps_the_last_item_and_items_keep_all() {
    let parsed = settings(
        "litellm_settings:\n  guardrails:\n    - dup:\n        callbacks: [first]\n    - other:\n        callbacks: [x]\n      dup:\n        callbacks: [second]\n",
    );
    let names: Vec<&str> = parsed
        .items
        .iter()
        .map(|item| item.guardrail_name.as_str())
        .collect();
    assert_eq!(names, ["dup", "other", "dup"]);
    assert_eq!(parsed.by_name.len(), 2);
    assert_eq!(parsed.by_name["dup"].callbacks, ["second"]);
    assert_eq!(parsed.by_name["other"].callbacks, ["x"]);
}

#[test]
fn legacy_config_loads_through_the_loader_without_unsupported_keys() {
    let source = "litellm_settings:\n  guardrails:\n    - pii:\n        callbacks: [presidio]\n        default_on: true\n        callback_args:\n          presidio:\n            language: en\n";
    let lookup = BTreeMap::<String, String>::new();
    let loaded = ConfigLoader { lookup }.load(source).unwrap();
    let map = &loaded.sections.legacy_guardrails.by_name;
    assert_eq!(map["pii"].callbacks, ["presidio"]);
}

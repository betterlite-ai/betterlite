#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::deployment_catalog::{GroupAlias, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.deployment_catalog)
        .map_err(|error| error.to_string())
}

fn alias(model: &str, hidden: bool) -> GroupAlias {
    GroupAlias {
        model: model.to_owned(),
        hidden,
    }
}

#[rstest]
#[case::no_router_settings("model_list: []")]
#[case::empty_router_settings("router_settings: {}")]
#[case::null_alias_map("router_settings:\n  model_group_alias:")]
fn model_group_alias_defaults_to_an_empty_map(#[case] source: &str) {
    assert_eq!(settings(source, &[]).unwrap(), Settings::default());
}

#[rstest]
#[case::plain_target("gpt: gpt-4", alias("gpt-4", false))]
#[case::item_target("gpt: {model: gpt-4, hidden: true}", alias("gpt-4", true))]
#[case::item_target_visible("gpt: {model: gpt-4, hidden: false}", alias("gpt-4", false))]
#[case::empty_target("gpt: ''", alias("", false))]
#[case::environment_reference("gpt: os.environ/TARGET", alias("gpt-4", false))]
#[case::environment_reference_in_item(
    "gpt: {model: os.environ/TARGET, hidden: true}",
    alias("gpt-4", true)
)]
#[case::unset_environment_reference("gpt: os.environ/MISSING", alias("", false))]
fn model_group_alias_reads_both_entry_shapes(#[case] entry: &str, #[case] expected: GroupAlias) {
    let source = format!("router_settings:\n  model_group_alias:\n    {entry}");
    let loaded = settings(&source, &[("TARGET", "gpt-4")]).unwrap();
    assert_eq!(loaded.model_group_alias.get("gpt"), Some(&expected));
}

#[test]
fn model_group_alias_keeps_the_declared_order() {
    let source = "router_settings:\n  model_group_alias:\n    zeta: a\n    alpha: b\n    mid: c";
    let loaded = settings(source, &[]).unwrap();
    let names: Vec<&str> = loaded
        .model_group_alias
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["zeta", "alpha", "mid"]);
}

#[test]
fn model_group_alias_names_an_unknown_item_key() {
    let source = "router_settings:\n  model_group_alias:\n    fast: {model: gpt-4, visible: true, hidden: false}\n    ok: {model: a, hidden: false}";
    let error = settings(source, &[]).unwrap_err();
    assert_eq!(
        error,
        r#"unsupported config keys: ["router_settings.model_group_alias.fast.visible"]"#
    );
}

#[rstest]
#[case::first_alias("fast")]
#[case::another_alias("smart")]
fn model_group_alias_requires_hidden_on_an_item(#[case] name: &str) {
    let source = format!("router_settings:\n  model_group_alias:\n    {name}: {{model: gpt-4}}");
    let error = settings(&source, &[]).unwrap_err();
    assert_eq!(
        error,
        format!(r#"unsupported config keys: ["router_settings.model_group_alias.{name}.hidden"]"#)
    );
}

#[test]
fn model_group_alias_reports_a_missing_hidden_beside_an_unknown_key() {
    let source = "router_settings:\n  model_group_alias:\n    fast: {model: gpt-4, visible: true}";
    let error = settings(source, &[]).unwrap_err();
    assert_eq!(
        error,
        r#"unsupported config keys: ["router_settings.model_group_alias.fast.visible", "router_settings.model_group_alias.fast.hidden"]"#
    );
}

#[rstest]
#[case::number_target("gpt: 4")]
#[case::list_target("gpt: [a]")]
#[case::item_without_model("gpt: {hidden: true}")]
#[case::hidden_not_a_bool("gpt: {model: a, hidden: maybe}")]
fn model_group_alias_rejects_a_wrong_typed_entry(#[case] entry: &str) {
    let source = format!("router_settings:\n  model_group_alias:\n    {entry}");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("model_group_alias"), "{error}");
}

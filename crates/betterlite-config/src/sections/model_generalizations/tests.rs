#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::json;

use crate::loader::ConfigLoader;
use crate::sections::model_generalizations::Settings;

fn settings(source: &str) -> Result<Settings, String> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.model_generalizations)
    .map_err(|error| error.to_string())
}

#[test]
fn an_absent_block_reads_no_rules() {
    let resolved = settings("model_list: []\n").unwrap();
    assert_eq!(resolved, Settings::default());
}

#[test]
fn rules_keep_their_document() {
    let source = "fallback_generalizations:\n  rules:\n    - pattern: \"^acme-\"\n      model_info:\n        litellm_provider: acme\n        mode: chat\n    - 5\n";
    let resolved = settings(source).unwrap();
    assert_eq!(
        resolved.rules,
        vec![
            json!({"pattern": "^acme-", "model_info": {"litellm_provider": "acme", "mode": "chat"}}),
            json!(5),
        ]
    );
}

#[test]
fn a_null_rule_list_reads_no_rules() {
    let resolved = settings("fallback_generalizations:\n  rules: null\n").unwrap();
    assert_eq!(resolved, Settings::default());
}

#[test]
fn a_non_list_rule_value_is_refused_with_its_path() {
    let error = settings("fallback_generalizations:\n  rules: text\n").unwrap_err();
    assert_eq!(error, "invalid scalar at fallback_generalizations.rules");
}

#[test]
fn an_unknown_block_key_is_unsupported() {
    let error = settings("fallback_generalizations:\n  extra: 1\n").unwrap_err();
    assert_eq!(
        error,
        r#"unsupported config keys: ["fallback_generalizations.extra"]"#
    );
}

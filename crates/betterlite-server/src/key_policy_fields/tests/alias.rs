#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::key_policy_fields::{
    BUILTIN_ALIAS_MESSAGE, CONFIGURED_ALIAS_MESSAGE, Settings,
};

use crate::key_policy_fields::alias::{INVALID_ALIAS_MESSAGE, validate_key_alias_format};

fn settings(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the alias settings load")
    .sections
    .key_policy_fields
}

fn builtin() -> Settings {
    settings("model_list: []\nlitellm_settings:\n  enable_key_alias_format_validation: true\n")
}

fn configured(pattern: &str) -> Settings {
    settings(&format!(
        "model_list: []\nlitellm_settings:\n  key_alias_pattern: '{pattern}'\n"
    ))
}

#[test]
fn unsafe_alias_always_rejected() {
    let messages: Vec<String> = ["a/../b", "a b", "a\u{7}b"]
        .iter()
        .map(|alias| {
            let error = validate_key_alias_format(Some(alias), None).unwrap_err();
            assert_eq!(error.code, "400");
            assert_eq!(error.error_type, "bad_request_error");
            assert_eq!(error.param.as_deref(), Some("key_alias"));
            error.message
        })
        .collect();
    assert_eq!(messages, vec![INVALID_ALIAS_MESSAGE.to_owned(); 3]);
    // the unsafe-name check runs before either format rule, whose message differs
    let builtin_settings = builtin();
    let error =
        validate_key_alias_format(Some("a b"), builtin_settings.alias_rule().as_ref()).unwrap_err();
    assert_eq!(error.message, INVALID_ALIAS_MESSAGE);
    let configured_settings = configured("[a-z ]+");
    let error = validate_key_alias_format(Some("a b"), configured_settings.alias_rule().as_ref())
        .unwrap_err();
    assert_eq!(error.message, INVALID_ALIAS_MESSAGE);
}

#[test]
fn builtin_alias_boundaries() {
    let loaded = builtin();
    let rule = loaded.alias_rule().expect("the toggle installs the rule");
    assert!(
        ["ab", "a_b", "a-b", "a/b", "a.b", "a@b", "A9"]
            .iter()
            .all(|alias| rule.accepts(alias))
    );
    assert!(
        ["a", "-ab", "ab-", "a b", "a!b", "aéb", "_ab"]
            .iter()
            .all(|alias| !rule.accepts(alias))
    );
    let longest = format!("a{}c", "b".repeat(253));
    assert_eq!(longest.len(), 255);
    assert!(rule.accepts(&longest));
    assert!(!rule.accepts(&format!("{longest}c")));
    validate_key_alias_format(Some("ab"), loaded.alias_rule().as_ref()).unwrap();
    let error = validate_key_alias_format(Some("-ab"), loaded.alias_rule().as_ref()).unwrap_err();
    assert_eq!(error.message, BUILTIN_ALIAS_MESSAGE);
    assert_eq!(error.code, "400");
}

#[test]
fn custom_fullmatch_and_length() {
    let alternation = configured("a|bc");
    let rule = alternation.alias_rule().expect("the pattern is the rule");
    // fullmatch rejects the concatenation a search would accept
    assert!(rule.accepts("a"));
    assert!(rule.accepts("bc"));
    assert!(!rule.accepts("bcbc"));
    assert!(!rule.accepts("abc"));
    let error =
        validate_key_alias_format(Some("abc"), alternation.alias_rule().as_ref()).unwrap_err();
    assert_eq!(error.message, format!("{CONFIGURED_ALIAS_MESSAGE}a|bc"));
    // the 255-character maximum applies to a custom rule as well
    let any = configured(".+");
    let rule = any.alias_rule().expect("the pattern is the rule");
    let longest = "a".repeat(255);
    assert!(rule.accepts(&longest));
    assert!(!rule.accepts(&"a".repeat(256)));
    assert_eq!(rule.message(), format!("{CONFIGURED_ALIAS_MESSAGE}.+"));
}

#[test]
fn validation_disabled_has_no_format_limit() {
    let loaded = settings("model_list: []\n");
    let rule = loaded.alias_rule();
    assert!(rule.is_none());
    let longest = "a".repeat(300);
    validate_key_alias_format(Some(&longest), None).unwrap();
    // the built-in rule would reject both of these
    validate_key_alias_format(Some("x"), None).unwrap();
    validate_key_alias_format(Some("-leading"), None).unwrap();
    validate_key_alias_format(None, None).unwrap();
}

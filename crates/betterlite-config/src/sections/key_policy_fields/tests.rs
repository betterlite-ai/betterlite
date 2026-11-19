#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::{ConfigError, ConfigLoader};

use super::{ALIAS_PATTERN_PATH, AliasRule, CONFIGURED_ALIAS_MESSAGE, Settings, VALIDATION_PATH};

fn lookup() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn settings(source: &str) -> Settings {
    ConfigLoader { lookup: lookup() }
        .load(source)
        .expect("the key policy settings load")
        .sections
        .key_policy_fields
}

#[test]
fn alias_defaults() {
    let loaded = settings("model_list: []\n");
    assert!(!loaded.alias_format_validation);
    assert!(loaded.alias_pattern.is_none());
    assert!(loaded.alias_rule().is_none());
}

#[test]
fn configured_pattern_overrides_toggle() {
    let loaded = settings(
        "litellm_settings:\n  enable_key_alias_format_validation: true\n  key_alias_pattern: 'key[+]v[0-9]'\n",
    );
    assert!(loaded.alias_format_validation);
    let Some(rule) = loaded.alias_rule() else {
        panic!("a configured pattern is a rule of its own");
    };
    assert!(matches!(&rule, AliasRule::Configured(_)));
    // the built-in rule rejects a plus, so accepting this alias proves the configured pattern ran
    assert!(rule.accepts("key+v1"));
    assert!(!rule.accepts("key-v1"));
    assert_eq!(
        rule.message(),
        format!("{CONFIGURED_ALIAS_MESSAGE}key[+]v[0-9]")
    );
}

// the toggle without a pattern installs the built-in character rule
#[test]
fn toggle_alone_installs_builtin() {
    let loaded = settings("litellm_settings:\n  enable_key_alias_format_validation: true\n");
    let Some(rule) = loaded.alias_rule() else {
        panic!("the toggle alone installs the built-in rule");
    };
    assert!(matches!(&rule, AliasRule::Builtin));
    assert!(rule.accepts("a_b"));
    assert!(!rule.accepts("a!b"));
    assert!(!rule.accepts("-ab"));
    assert!(!rule.accepts("ab-"));
    assert!(rule.accepts(&"a".repeat(255)));
    assert!(!rule.accepts(&"a".repeat(256)));
}

#[test]
fn empty_pattern_is_configured() {
    let loaded = settings(
        "litellm_settings:\n  enable_key_alias_format_validation: true\n  key_alias_pattern: \"\"\n",
    );
    let Some(rule) = loaded.alias_rule() else {
        panic!("an empty pattern is still a configured pattern");
    };
    assert!(matches!(&rule, AliasRule::Configured(_)));
    assert!(rule.accepts(""));
    assert!(!rule.accepts("sk-alias"));
    assert!(rule.message().ends_with("key_alias_pattern: "));
}

#[test]
fn nonstring_pattern_rejected() {
    let loaded =
        ConfigLoader { lookup: lookup() }.load("litellm_settings:\n  key_alias_pattern: 42\n");
    let error = match loaded {
        Ok(_config) => panic!("a numeric pattern must not load"),
        Err(error) => error,
    };
    match error {
        ConfigError::InvalidScalar { path, .. } => assert_eq!(path, ALIAS_PATTERN_PATH),
        other @ (ConfigError::CustomDimensions(_)
        | ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::Unsupported { .. }
        | ConfigError::MissingEnvironment { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. }
        | ConfigError::LoadThread { .. }) => {
            panic!("expected an invalid scalar, got: {other}")
        }
    }
}

#[test]
fn pattern_from_environment() {
    let loaded = ConfigLoader {
        lookup: BTreeMap::from([("KEY_ALIAS_PATTERN".to_owned(), "key[+]v[0-9]".to_owned())]),
    }
    .load("litellm_settings:\n  key_alias_pattern: os.environ/KEY_ALIAS_PATTERN\n")
    .expect("the env pattern loads")
    .sections
    .key_policy_fields;
    let Some(rule) = loaded.alias_rule() else {
        panic!("the resolved pattern is the rule");
    };
    // the built-in rule rejects a plus, so accepting this alias proves the resolved pattern ran
    assert!(rule.accepts("key+v1"));
    assert!(!rule.accepts("key-v1"));
    assert_eq!(
        rule.message(),
        format!("{CONFIGURED_ALIAS_MESSAGE}key[+]v[0-9]")
    );
    let missing = ConfigLoader { lookup: lookup() }
        .load("litellm_settings:\n  key_alias_pattern: os.environ/KEY_ALIAS_PATTERN\n");
    match missing {
        Ok(_config) => panic!("a missing variable must not load"),
        Err(ConfigError::MissingEnvironment { variable, path }) => {
            assert_eq!(variable, "KEY_ALIAS_PATTERN");
            assert_eq!(path, ALIAS_PATTERN_PATH);
        }
        Err(other) => panic!("expected a missing environment, got: {other}"),
    }
}

#[test]
fn validation_from_environment() {
    let source =
        "litellm_settings:\n  enable_key_alias_format_validation: os.environ/ALIAS_VALIDATION\n";
    // the shared bool parser trims the text and reads yes
    let enabled = ConfigLoader {
        lookup: BTreeMap::from([("ALIAS_VALIDATION".to_owned(), " YES ".to_owned())]),
    }
    .load(source)
    .expect("the env toggle loads");
    assert!(enabled.sections.key_policy_fields.alias_format_validation);
    // an empty variable counts as unset and takes the default
    let empty = ConfigLoader {
        lookup: BTreeMap::from([("ALIAS_VALIDATION".to_owned(), String::new())]),
    }
    .load(source)
    .expect("an empty env counts as unset");
    assert!(!empty.sections.key_policy_fields.alias_format_validation);
    let malformed = ConfigLoader {
        lookup: BTreeMap::from([("ALIAS_VALIDATION".to_owned(), "maybe".to_owned())]),
    }
    .load(source);
    assert!(matches!(malformed, Err(ConfigError::Environment { .. })));
    let missing = ConfigLoader { lookup: lookup() }.load(source);
    match missing {
        Ok(_config) => panic!("a missing variable must not load"),
        Err(ConfigError::MissingEnvironment { variable, path }) => {
            assert_eq!(variable, "ALIAS_VALIDATION");
            assert_eq!(path, VALIDATION_PATH);
        }
        Err(other) => panic!("expected a missing environment, got: {other}"),
    }
    // literal text where a bool is declared is still an error that names the field
    let literal = ConfigLoader { lookup: lookup() }
        .load("litellm_settings:\n  enable_key_alias_format_validation: 'true'\n");
    assert!(matches!(
        literal,
        Err(ConfigError::InvalidScalar { path, .. }) if path == VALIDATION_PATH
    ));
}

#[test]
fn invalid_regex_rejected() {
    // "(" is unbalanced on its own; "x)|(.*" only fails the bare compile, since the anchored wrapper
    // would otherwise accept it as an alternation that matches almost any alias
    for source in ["(", "x)|(.*"] {
        let document = format!("litellm_settings:\n  key_alias_pattern: '{source}'\n");
        let loaded = ConfigLoader { lookup: lookup() }.load(&document);
        let error = match loaded {
            Ok(_config) => panic!("{source} must not load"),
            Err(error) => error,
        };
        match error {
            ConfigError::InvalidScalar { path, .. } => assert_eq!(path, ALIAS_PATTERN_PATH),
            other @ (ConfigError::CustomDimensions(_)
            | ConfigError::Environment { .. }
            | ConfigError::Yaml { .. }
            | ConfigError::Unsupported { .. }
            | ConfigError::MissingEnvironment { .. }
            | ConfigError::InvalidFloat { .. }
            | ConfigError::InvalidInteger { .. }
            | ConfigError::LoadThread { .. }) => {
                panic!("expected an invalid scalar, got: {other}")
            }
        }
    }
}

#[test]
fn stored_rotation_alias() {
    let default = ConfigLoader { lookup: lookup() }
        .load("model_list: []\n")
        .expect("the default config loads");
    assert!(!default.sections.key_management_settings.store_virtual_keys);
    let configured = ConfigLoader {
        lookup: BTreeMap::from([("STORE_VIRTUAL_KEYS".to_owned(), "true".to_owned())]),
    }
    .load(
        "general_settings:\n  key_management_settings:\n    store_virtual_keys: os.environ/STORE_VIRTUAL_KEYS\n",
    )
    .expect("the stored-key config loads");
    assert!(
        configured
            .sections
            .key_management_settings
            .store_virtual_keys
    );
    let misplaced = ConfigLoader { lookup: lookup() }
        .load("litellm_settings:\n  key_management_settings:\n    store_virtual_keys: true\n");
    match misplaced {
        Ok(_config) => panic!("the setting belongs under general_settings"),
        Err(ConfigError::Unsupported { paths }) => assert_eq!(
            paths,
            ["litellm_settings.key_management_settings".to_owned()]
        ),
        Err(other) => panic!("expected an unsupported key, got: {other}"),
    }
}

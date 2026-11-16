#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::database_reencryption::RECURSE_DEPTH_VARIABLE;
use crate::types::Config;

fn load(environment: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }.load("{}")
}

fn loaded_depth(environment: &[(&str, &str)]) -> Result<i64, ConfigError> {
    load(environment).map(|config| config.sections.database_reencryption.max_recurse_depth)
}

// the depth the config loads is the one the auth model pattern check enforces
fn decision_at(depth: i64, fallback_depth: i64) -> ModelAccessDecision {
    let aliases = BTreeMap::new();
    let provider = UnresolvedProviderInference;
    let context = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: depth,
    };
    let grant = ModelGrantPattern {
        models: vec!["gpt-4o".to_owned()],
        team_model_aliases: BTreeMap::new(),
        key_model_aliases: BTreeMap::new(),
        team_id: None,
    };
    can_object_call_model(
        &RequestedModels::One("gpt-4o".to_owned()),
        &grant,
        ModelAccessObject::Key,
        &context,
        fallback_depth,
    )
}

#[test]
fn loads_settings() {
    let depth = loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "7")]).expect("the setting loads");
    assert_eq!(depth, 7);
    // the loaded value is what the model pattern check enforces, not a copy of the default
    assert!(decision_at(depth, 6).is_allowed());
    assert_eq!(
        decision_at(depth, 7),
        ModelAccessDecision::RecurseDepthExceeded {
            model: "gpt-4o".to_owned()
        }
    );
}

#[test]
fn defaults_and_precedence() {
    // the pinned default of DEFAULT_MAX_RECURSE_DEPTH is 100
    assert_eq!(loaded_depth(&[]).expect("the default loads"), 100);
    assert!(decision_at(100, 99).is_allowed());
    assert!(matches!(
        decision_at(100, 100),
        ModelAccessDecision::RecurseDepthExceeded { .. }
    ));

    // an empty environment value counts as unset, so the default stands
    assert_eq!(
        loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "")]).expect("the empty setting loads"),
        100
    );
    assert_eq!(
        loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "   ")]).expect("the blank setting loads"),
        100
    );

    // the environment value takes precedence over the default
    let configured = loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "3")]).expect("the setting loads");
    assert_eq!(configured, 3);
    assert!(decision_at(configured, 2).is_allowed());
    assert!(matches!(
        decision_at(configured, 3),
        ModelAccessDecision::RecurseDepthExceeded { .. }
    ));
}

#[test]
fn invalid_value_fallback() {
    // a non-numeric value is an error naming the binding, never a silent default
    let Err(error) = loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "not-a-number")]) else {
        panic!("a non-numeric depth must not load");
    };
    assert!(
        error.to_string().contains(RECURSE_DEPTH_VARIABLE),
        "the error names the binding: {error}"
    );

    // a negative integer is a value int() accepts, so it loads as given
    let negative = loaded_depth(&[(RECURSE_DEPTH_VARIABLE, "-1")]).expect("a negative depth loads");
    assert_eq!(negative, -1);
    assert!(matches!(
        decision_at(negative, 0),
        ModelAccessDecision::RecurseDepthExceeded { .. }
    ));
}

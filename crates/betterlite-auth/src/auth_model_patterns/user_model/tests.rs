#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use rstest::rstest;

use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::denial::{ModelAccessDecision, model_denied_client_message};
use crate::auth_model_patterns::pattern::{ProviderInference, UnresolvedProviderInference};
use crate::auth_model_patterns::user_model::can_user_call_model;
use crate::auth_request_models::RequestedModels;

const RECURSE_DEPTH: i64 = 100;

fn user(models: &[&str]) -> LiteLLMUserTable {
    let record = JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String("user-1".to_owned())),
        (
            "models".to_owned(),
            JsonData::Array(
                models
                    .iter()
                    .map(|name| JsonData::String((*name).to_owned()))
                    .collect(),
            ),
        ),
    ]));
    user_record_to_model(Some(&record))
        .unwrap()
        .expect("the fixture user row is present")
}

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn many(models: &[&str]) -> RequestedModels {
    RequestedModels::Many(models.iter().map(|model| (*model).to_owned()).collect())
}

fn empty_aliases() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn context<'context>(
    aliases: &'context BTreeMap<String, String>,
    provider: &'context dyn ProviderInference,
) -> ModelAccessContext<'context> {
    ModelAccessContext {
        view: None,
        global_model_aliases: aliases,
        compaction: None,
        provider,
        max_recurse_depth: RECURSE_DEPTH,
    }
}

fn denial_of(decision: ModelAccessDecision) -> ModelAccessDeniedProxyException {
    match decision {
        ModelAccessDecision::Denied(denial) => *denial,
        other @ (ModelAccessDecision::Allowed
        | ModelAccessDecision::RecurseDepthExceeded { .. }) => {
            panic!("expected a denial, got {other:?}")
        }
    }
}

#[test]
fn absent_user_allows() {
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("private-model"), None, &empty_aliases(), &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

#[test]
fn no_default_models_uses_key_type() {
    let row = user(&["no-default-models"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("gpt-4o"), Some(&row), &empty_aliases(), &ctx);

    let denial = denial_of(decision);
    let exception = denial.as_proxy_exception();
    assert_eq!(
        exception.error_type,
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");
    assert_eq!(exception.message, model_denied_client_message("gpt-4o"));
    assert_eq!(
        denial.sanitized_internal_message(),
        "User not allowed to access model. No default model access, only team models allowed. \
         Tried to access gpt-4o"
    );
}

#[test]
fn listed_model_allowed() {
    let row = user(&["gpt-4o", "o3"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("o3"), Some(&row), &empty_aliases(), &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

#[test]
fn unlisted_model_uses_user_type() {
    let row = user(&["gpt-4o"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("o3"), Some(&row), &empty_aliases(), &ctx);

    let denial = denial_of(decision);
    let exception = denial.as_proxy_exception();
    assert_eq!(
        exception.error_type,
        ProxyErrorTypes::UserModelAccessDenied.to_string()
    );
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");
    assert_eq!(exception.message, model_denied_client_message("o3"));
}

#[test]
fn empty_user_list_allows_all() {
    let row = user(&[]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("private-model"), Some(&row), &empty_aliases(), &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

#[test]
fn key_alias_target_checked() {
    let row = user(&["gpt-4o"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);
    let key_aliases = BTreeMap::from([("fast".to_owned(), "gpt-4o".to_owned())]);

    let decision = can_user_call_model(&one("fast"), Some(&row), &key_aliases, &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

#[test]
fn list_stops_at_first_denial() {
    let row = user(&["gpt-4o"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision =
        can_user_call_model(&many(&["o3", "gpt-4o"]), Some(&row), &empty_aliases(), &ctx);

    let denial = denial_of(decision);
    let exception = denial.as_proxy_exception();
    assert_eq!(
        exception.error_type,
        ProxyErrorTypes::UserModelAccessDenied.to_string()
    );
    assert_eq!(exception.message, model_denied_client_message("o3"));
}

// the sentinel is tested before any alias or pattern runs, so a listed model does not save the user
#[test]
fn sentinel_denies_even_when_model_listed() {
    let row = user(&["no-default-models", "gpt-4o"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("gpt-4o"), Some(&row), &empty_aliases(), &ctx);

    let denial = denial_of(decision);
    assert_eq!(
        denial.as_proxy_exception().error_type,
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
}

#[test]
fn sentinel_message_joins_list_models() {
    let row = user(&["no-default-models"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision =
        can_user_call_model(&many(&["gpt-4o", "o3"]), Some(&row), &empty_aliases(), &ctx);

    let denial = denial_of(decision);
    assert_eq!(
        denial.sanitized_internal_message(),
        "User not allowed to access model. No default model access, only team models allowed. \
         Tried to access gpt-4o,o3"
    );
}

// docs/deviations/B10051.yaml: B871's projection empties the column on one non-string item, even around the sentinel
#[test]
fn invalid_model_column_treated_as_empty() {
    let numeric: LiteLLMUserTable =
        serde_json::from_str(r#"{"user_id": "user-1", "models": [7]}"#).unwrap();
    let sentinel_and_number: LiteLLMUserTable =
        serde_json::from_str(r#"{"user_id": "user-2", "models": ["no-default-models", 7]}"#)
            .unwrap();
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let numeric_decision = can_user_call_model(
        &one("private-model"),
        Some(&numeric),
        &empty_aliases(),
        &ctx,
    );
    let mixed_decision = can_user_call_model(
        &one("gpt-4o"),
        Some(&sentinel_and_number),
        &empty_aliases(),
        &ctx,
    );

    assert_eq!(numeric_decision, ModelAccessDecision::Allowed);
    assert_eq!(mixed_decision, ModelAccessDecision::Allowed);
}

#[rstest]
#[case::star("*")]
#[case::prefix("gpt-*")]
#[case::all_proxy("all-proxy-models")]
fn wildcard_allowlist_grants(#[case] allowed: &str) {
    let row = user(&[allowed]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = context(&aliases, &provider);

    let decision = can_user_call_model(&one("gpt-4o"), Some(&row), &empty_aliases(), &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

// LiteLLM starts the object check at fallback depth 0; a budget of one still lets the first model through
#[test]
fn check_starts_at_depth_zero() {
    let row = user(&["gpt-4o"]);
    let aliases = empty_aliases();
    let provider = UnresolvedProviderInference;
    let ctx = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: 1,
    };

    let decision = can_user_call_model(&one("gpt-4o"), Some(&row), &empty_aliases(), &ctx);

    assert_eq!(decision, ModelAccessDecision::Allowed);
}

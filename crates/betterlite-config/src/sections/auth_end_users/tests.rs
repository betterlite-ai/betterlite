#![cfg(test)]

use std::collections::BTreeMap;

use super::Settings;
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.auth_end_users)
        .map_err(|error| error.to_string())
}

#[test]
fn max_end_user_default_unset() {
    let absent = settings("model_list: []", &[]).expect("an absent section loads");
    assert_eq!(absent, Settings::default());
    assert_eq!(absent.max_end_user_budget_id, None);
    let null = settings("litellm_settings:\n  max_end_user_budget_id: null", &[])
        .expect("null is the default");
    assert_eq!(null.max_end_user_budget_id, None);
    // an empty environment variable counts as unset
    let empty = settings(
        "litellm_settings:\n  max_end_user_budget_id: os.environ/END_USER_BUDGET",
        &[("END_USER_BUDGET", "")],
    )
    .expect("an empty reference is unset");
    assert_eq!(empty.max_end_user_budget_id, None);
    // a non-string is a field error naming the setting, not a silent default
    let wrong = settings("litellm_settings:\n  max_end_user_budget_id: 12", &[])
        .expect_err("a number is not an id");
    assert!(
        wrong.contains("litellm_settings.max_end_user_budget_id"),
        "{wrong}"
    );
}

#[test]
fn max_end_user_id_loaded() {
    let configured = settings(
        "litellm_settings:\n  max_end_user_budget_id: budget-global",
        &[],
    )
    .expect("the budget loads");
    assert_eq!(
        configured.max_end_user_budget_id.as_deref(),
        Some("budget-global")
    );
    let from_environment = settings(
        "litellm_settings:\n  max_end_user_budget_id: os.environ/END_USER_BUDGET",
        &[("END_USER_BUDGET", "budget-env")],
    )
    .expect("the reference resolves");
    assert_eq!(
        from_environment.max_end_user_budget_id.as_deref(),
        Some("budget-env")
    );
    // an explicit empty string is the value, so it overrides the proxy default with nothing
    let empty = settings("litellm_settings:\n  max_end_user_budget_id: ''", &[])
        .expect("an empty text loads");
    assert_eq!(empty.max_end_user_budget_id.as_deref(), Some(""));
}

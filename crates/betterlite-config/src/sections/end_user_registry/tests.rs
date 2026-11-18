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
        .map(|config| config.sections.end_user_registry)
        .map_err(|error| error.to_string())
}

#[test]
fn default_optimization_enabled() {
    // absent keys take the pinned litellm defaults, so the registry-driven skip stays on
    let absent = settings("model_list: []", &[]).expect("an absent section loads");
    assert_eq!(
        absent,
        Settings {
            validate_end_user_id_in_db: false,
            max_end_user_budget_id: None,
        }
    );
    // the same answer spelled out, next to unrelated litellm settings the loader still accepts
    let explicit = settings(
        "litellm_settings:\n  num_retries: 2\n  validate_end_user_id_in_db: false\n  max_end_user_budget_id: null\n",
        &[],
    )
    .expect("explicit defaults load");
    assert_eq!(explicit, absent);
}

#[test]
fn validation_disables_skip() {
    let disabled = settings("litellm_settings:\n  validate_end_user_id_in_db: true", &[])
        .expect("the flag loads");
    assert!(disabled.validate_end_user_id_in_db);
    // a text value goes through the shared parser, which reads 1, yes and on as well as true
    let text = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: 'true'",
        &[],
    )
    .expect("the text form loads");
    assert!(text.validate_end_user_id_in_db);
    let environment = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: os.environ/VALIDATE_END_USERS",
        &[("VALIDATE_END_USERS", "1")],
    )
    .expect("the reference resolves");
    assert!(environment.validate_end_user_id_in_db);
    let off = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: os.environ/VALIDATE_END_USERS",
        &[("VALIDATE_END_USERS", "off")],
    )
    .expect("the false text loads");
    assert!(!off.validate_end_user_id_in_db);
    // an empty value counts as unset, so the pinned false default stays
    let empty = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: os.environ/VALIDATE_END_USERS",
        &[("VALIDATE_END_USERS", "")],
    )
    .expect("an empty reference is unset");
    assert!(!empty.validate_end_user_id_in_db);
    // an unset reference names both the variable and the setting
    let missing = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: os.environ/VALIDATE_END_USERS",
        &[],
    )
    .expect_err("an unset reference fails the load");
    assert!(missing.contains("VALIDATE_END_USERS"), "{missing}");
    assert!(
        missing.contains("litellm_settings.validate_end_user_id_in_db"),
        "{missing}"
    );
    // unreadable text is a load error naming the setting
    let unreadable = settings(
        "litellm_settings:\n  validate_end_user_id_in_db: sometimes",
        &[],
    )
    .expect_err("text the parser cannot read fails the load");
    assert!(
        unreadable.contains("litellm_settings.validate_end_user_id_in_db"),
        "{unreadable}"
    );
    // any other type is a field error naming the setting, not a silent false
    let wrong = settings("litellm_settings:\n  validate_end_user_id_in_db: 1", &[])
        .expect_err("a number is not a flag");
    assert!(
        wrong.contains("litellm_settings.validate_end_user_id_in_db"),
        "{wrong}"
    );
}

#[test]
fn default_budget_disables_skip() {
    let configured = settings(
        "litellm_settings:\n  max_end_user_budget_id: budget-global",
        &[],
    )
    .expect("the budget loads");
    assert_eq!(
        configured.max_end_user_budget_id.as_deref(),
        Some("budget-global")
    );
    let resolved = settings(
        "litellm_settings:\n  max_end_user_budget_id: os.environ/END_USER_BUDGET",
        &[("END_USER_BUDGET", "budget-env")],
    )
    .expect("the reference resolves");
    assert_eq!(
        resolved.max_end_user_budget_id.as_deref(),
        Some("budget-env")
    );
    // an empty literal is used as given, so it still disables the skip
    let empty = settings("litellm_settings:\n  max_end_user_budget_id: ''", &[])
        .expect("an empty text loads");
    assert_eq!(empty.max_end_user_budget_id.as_deref(), Some(""));
    // an empty environment variable counts as unset, so the skip stays on
    let unset = settings(
        "litellm_settings:\n  max_end_user_budget_id: os.environ/END_USER_BUDGET",
        &[("END_USER_BUDGET", "")],
    )
    .expect("an empty reference is unset");
    assert_eq!(unset.max_end_user_budget_id, None);
    let null = settings("litellm_settings:\n  max_end_user_budget_id: null", &[])
        .expect("null is the default");
    assert_eq!(null.max_end_user_budget_id, None);
    // an unset reference names both the variable and the setting
    let missing = settings(
        "litellm_settings:\n  max_end_user_budget_id: os.environ/END_USER_BUDGET",
        &[],
    )
    .expect_err("an unset reference fails the load");
    assert!(missing.contains("END_USER_BUDGET"), "{missing}");
    assert!(
        missing.contains("litellm_settings.max_end_user_budget_id"),
        "{missing}"
    );
    // a non-string is a field error naming the setting
    let wrong = settings("litellm_settings:\n  max_end_user_budget_id: 12", &[])
        .expect_err("a number is not an id");
    assert!(
        wrong.contains("litellm_settings.max_end_user_budget_id"),
        "{wrong}"
    );
}

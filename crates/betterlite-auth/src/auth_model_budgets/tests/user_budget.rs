use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde_json::Number;

use crate::auth_model_budgets::checks::{
    check_team_model_budget, check_user_model_budget, read_user_model_max_budget,
};
use crate::auth_model_budgets::tests::fixtures::{
    ScriptedTeamLimiter, ScriptedUserLimiter, budget_map, other_error, text, token,
};
use crate::auth_model_budgets::tests::logs::Logs;
use crate::auth_model_budgets::tests::support::Harness;

fn objects() -> JsonData {
    JsonData::Object(BTreeMap::new())
}

#[tokio::test]
async fn user_lookup_skip_no_upsert_and_debug_failure() {
    let logs = Logs::default();
    let _guard = logs.install();
    let harness = Harness::default();
    let lookup = harness.lookup();
    assert_eq!(read_user_model_max_budget(None, Some("u1")).await, None);
    assert_eq!(read_user_model_max_budget(Some(&lookup), None).await, None);
    assert!(harness.store.finds.lock().unwrap().is_empty());
    // a missing row is not created because the lookup passes user_id_upsert=False
    assert_eq!(
        read_user_model_max_budget(Some(&lookup), Some("missing")).await,
        None
    );
    assert_eq!(
        *harness.store.finds.lock().unwrap(),
        vec!["missing".to_owned()]
    );
    assert!(harness.store.creates.lock().unwrap().is_empty());
    assert_eq!(
        logs.debugs("Unable to read user for the per-model budget check")
            .len(),
        1
    );
    *harness.store.row.lock().unwrap() = Some(JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), text("u2")),
        ("model_max_budget".to_owned(), budget_map(&[("gpt-4", 5)])),
    ])));
    let found = read_user_model_max_budget(Some(&lookup), Some("u2"))
        .await
        .expect("the row carries a map");
    assert_eq!(found.get("gpt-4"), Some(&JsonData::Number(Number::from(5))));
    assert!(harness.store.creates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn user_empty_budget_skips() {
    let limiter = ScriptedUserLimiter::default();
    let models = vec!["gpt-4".to_owned()];
    let empty = token(&[
        ("user_id", text("u1")),
        ("user_model_max_budget", objects()),
    ]);
    check_user_model_budget(&limiter, &empty, &models)
        .await
        .unwrap();
    let absent = token(&[("user_id", text("u1"))]);
    check_user_model_budget(&limiter, &absent, &models)
        .await
        .unwrap();
    let no_user = token(&[("user_model_max_budget", budget_map(&[("gpt-4", 5)]))]);
    check_user_model_budget(&limiter, &no_user, &models)
        .await
        .unwrap();
    assert!(limiter.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn user_and_team_models_ordered_duplicates() {
    let user = ScriptedUserLimiter::default();
    let team = ScriptedTeamLimiter::default();
    let key = token(&[
        ("user_id", text("u1")),
        ("team_id", text("t1")),
        ("user_model_max_budget", budget_map(&[("gpt-4", 5)])),
        ("team_model_max_budget", budget_map(&[("gpt-4", 6)])),
        ("model_max_budget", budget_map(&[("gpt-4", 7)])),
    ]);
    let models = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
    check_user_model_budget(&user, &key, &models).await.unwrap();
    check_team_model_budget(&team, &key, &models).await.unwrap();
    assert_eq!(
        *user.calls.lock().unwrap(),
        vec![
            ("u1".to_owned(), "a".to_owned()),
            ("u1".to_owned(), "b".to_owned()),
            ("u1".to_owned(), "a".to_owned())
        ]
    );
    let team_seen = team.seen.lock().unwrap();
    let team_models: Vec<&str> = team_seen.iter().map(|seen| seen.model.as_str()).collect();
    assert_eq!(team_models, vec!["a", "b", "a"]);
    assert!(team_seen.iter().all(|seen| seen.team_id == "t1"));
    assert!(team_seen.iter().all(|seen| seen.budget.len() == 1));
    assert!(team_seen.iter().all(|seen| {
        seen.key_budget
            .as_ref()
            .is_some_and(|budget| budget.len() == 1)
    }));
    let seen_budget = user.seen.lock().unwrap()[0].1.clone();
    assert_eq!(
        seen_budget,
        BTreeMap::from([("gpt-4".to_owned(), JsonData::Number(Number::from(5))),])
    );
}

#[tokio::test]
async fn team_key_override_map_passthrough() {
    let team = ScriptedTeamLimiter::default();
    let key = token(&[
        ("team_id", text("t1")),
        ("team_model_max_budget", budget_map(&[("gpt-4", 6)])),
        ("model_max_budget", budget_map(&[("gpt-4", 7)])),
    ]);
    check_team_model_budget(&team, &key, &["gpt-4".to_owned()])
        .await
        .unwrap();
    let expected = BTreeMap::from([("gpt-4".to_owned(), JsonData::Number(Number::from(7)))]);
    let seen = team.seen.lock().unwrap();
    assert_eq!(seen[0].key_budget, Some(expected));
    assert_eq!(
        seen[0].budget,
        BTreeMap::from([("gpt-4".to_owned(), JsonData::Number(Number::from(6)))])
    );
}

#[tokio::test]
async fn limiter_false_return_is_not_denial() {
    // both default fixtures answer Ok(false), and a false answer is not a denial
    let user = ScriptedUserLimiter::default();
    let team = ScriptedTeamLimiter::default();
    let key = token(&[
        ("user_id", text("u1")),
        ("team_id", text("t1")),
        ("user_model_max_budget", budget_map(&[("gpt-4", 5)])),
        ("team_model_max_budget", budget_map(&[("gpt-4", 6)])),
    ]);
    let models = vec!["gpt-4".to_owned()];
    check_user_model_budget(&user, &key, &models).await.unwrap();
    check_team_model_budget(&team, &key, &models).await.unwrap();
    assert_eq!(user.calls.lock().unwrap().len(), 1);
    assert_eq!(team.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn user_and_team_errors_propagate() {
    let user = ScriptedUserLimiter::default();
    *user.failure.lock().unwrap() = Some(other_error());
    let key = token(&[
        ("user_id", text("u1")),
        ("user_model_max_budget", budget_map(&[("gpt-4", 5)])),
    ]);
    let error = check_user_model_budget(&user, &key, &["gpt-4".to_owned()])
        .await
        .expect_err("a limiter failure stops the model loop");
    assert_eq!(error, other_error());
}

#[tokio::test]
async fn absent_database_skips_even_a_cached_user() {
    let mut harness = Harness::default();
    harness.with_store = false;
    *harness.cache.cached.lock().unwrap() = Some(
        serde_json::from_str(r#"{"user_id":"cached","model_max_budget":{"gpt-4":3}}"#).unwrap(),
    );
    let lookup = harness.lookup();
    assert_eq!(
        read_user_model_max_budget(Some(&lookup), Some("cached")).await,
        None
    );
    assert!(harness.cache.writes.lock().unwrap().is_empty());
}

#[rstest::rstest]
#[case::no_identity(vec![("team_model_max_budget", budget_map(&[("model", 1)]))])]
#[case::no_budget(vec![("team_id", text("team"))])]
#[case::empty_budget(vec![("team_id", text("team")), ("team_model_max_budget", objects())])]
#[tokio::test]
async fn team_missing_identity_or_budget_skips(#[case] fields: Vec<(&str, JsonData)>) {
    let limiter = ScriptedTeamLimiter::default();
    check_team_model_budget(&limiter, &token(&fields), &["model".to_owned()])
        .await
        .unwrap();
    assert!(limiter.seen.lock().unwrap().is_empty());
}

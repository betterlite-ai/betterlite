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
        vec!["u1:a".to_owned(), "u1:b".to_owned(), "u1:a".to_owned()]
    );
    assert_eq!(
        *team.calls.lock().unwrap(),
        vec![
            "t1:a:1".to_owned(),
            "t1:b:1".to_owned(),
            "t1:a:1".to_owned()
        ]
    );
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
    assert_eq!(team.key_budgets.lock().unwrap()[0], Some(expected));
}

#[tokio::test]
async fn limiter_false_return_is_not_denial() {
    let user = ScriptedUserLimiter {
        within: false,
        ..ScriptedUserLimiter::default()
    };
    let team = ScriptedTeamLimiter {
        within: false,
        ..ScriptedTeamLimiter::default()
    };
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
    assert_eq!(team.calls.lock().unwrap().len(), 1);
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

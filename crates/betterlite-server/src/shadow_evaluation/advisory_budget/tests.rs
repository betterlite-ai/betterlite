#![cfg(test)]

use betterlite_auth::auth_key_budgets::alerts::BudgetAlertKind;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::budget_primitives::window_start::DurationParser as _;
use betterlite_auth::team_budget_admission::alerts::TeamAlertKind;
use betterlite_auth::team_budget_admission::sources::MembershipSource as _;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{AdvisoryDeps, NoSources, key_or_team_over_budget};
use crate::shadow_evaluation::tests::{
    Alerts, SpendFake, SpendRead, SpendScript, TeamAnswer, TeamsFake, add_field, decimal,
    hook_metadata, json, key_auth, now, team_row,
};

struct Harness {
    spend: SpendFake,
    teams: TeamsFake,
    alerts: Alerts,
}

impl Harness {
    fn new(key: SpendScript, team: SpendScript) -> Self {
        Self {
            spend: SpendFake::new(key, team),
            teams: TeamsFake::new(TeamAnswer::NoDatabase),
            alerts: Alerts::default(),
        }
    }

    fn deps(&self) -> AdvisoryDeps<'_> {
        AdvisoryDeps {
            spend: &self.spend,
            teams: &self.teams,
            key_alerts: Some(&self.alerts),
            team_alerts: Some(&self.alerts),
            now: now(),
        }
    }
}

#[tokio::test]
async fn a_missing_auth_record_is_not_over_budget() {
    let harness = Harness::new(
        SpendScript::Value(decimal("99")),
        SpendScript::Value(decimal("99")),
    );
    assert!(!key_or_team_over_budget(&IndexMap::new(), &harness.deps()).await);
    assert!(harness.spend.reads().is_empty());
    assert!(harness.teams.lookups().is_empty());
}

#[tokio::test]
async fn a_non_object_auth_record_is_not_over_budget() {
    let harness = Harness::new(
        SpendScript::Value(decimal("99")),
        SpendScript::Value(decimal("99")),
    );
    let metadata = hook_metadata(Json::Text("sk-not-a-record".to_owned()));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert!(harness.spend.reads().is_empty());
}

#[tokio::test]
async fn an_undecodable_auth_record_is_not_over_budget() {
    let harness = Harness::new(
        SpendScript::Value(decimal("99")),
        SpendScript::Value(decimal("99")),
    );
    let metadata = hook_metadata(json(r#"{"api_key":5}"#));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert!(harness.spend.reads().is_empty());
    assert!(harness.teams.lookups().is_empty());
}

#[tokio::test]
async fn a_key_at_its_limit_is_over_budget_and_skips_the_team() {
    let harness = Harness::new(SpendScript::Value(decimal("10")), SpendScript::Unreadable);
    let metadata = hook_metadata(key_auth(Some("10"), "10", Some("team-1")));
    assert!(key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(
        harness.spend.reads(),
        vec![SpendRead {
            counter_key: "spend:key:token-1".to_owned(),
            fallback_spend: decimal("10"),
            max_budget: Some(decimal("10")),
        }]
    );
    assert!(harness.teams.lookups().is_empty());
    let alerts = harness.alerts.budget_alerts();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, BudgetAlertKind::TokenBudget);
    assert_eq!(alerts[0].spend, decimal("10"));
    assert_eq!(alerts[0].max_budget, Some(Money::Finite(decimal("10"))));
}

#[tokio::test]
async fn a_key_without_a_maximum_skips_the_key_read() {
    let mut harness = Harness::new(SpendScript::Unreadable, SpendScript::Value(decimal("1")));
    harness.teams = TeamsFake::new(TeamAnswer::Found(Box::new(team_row("team-1", "5", "1"))));
    let metadata = hook_metadata(key_auth(None, "9", Some("team-1")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(
        harness.spend.reads(),
        vec![SpendRead {
            counter_key: "spend:team:team-1".to_owned(),
            fallback_spend: decimal("1"),
            max_budget: Some(decimal("5")),
        }]
    );
}

#[tokio::test]
async fn a_key_below_its_limit_checks_the_team() {
    let mut harness = Harness::new(
        SpendScript::Value(decimal("9.5")),
        SpendScript::Value(decimal("5")),
    );
    harness.teams = TeamsFake::new(TeamAnswer::Found(Box::new(team_row("team-1", "5", "5"))));
    let metadata = hook_metadata(key_auth(Some("10"), "9.5", Some("team-1")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(
        harness.spend.reads(),
        vec![
            SpendRead {
                counter_key: "spend:key:token-1".to_owned(),
                fallback_spend: decimal("9.5"),
                max_budget: Some(decimal("10")),
            },
            SpendRead {
                counter_key: "spend:team:team-1".to_owned(),
                fallback_spend: decimal("5"),
                max_budget: Some(decimal("5")),
            },
        ]
    );
    assert_eq!(harness.teams.lookups(), vec!["team-1".to_owned()]);
    assert!(harness.alerts.team_alerts().is_empty());
}

#[tokio::test]
async fn a_team_above_its_limit_is_over_budget() {
    let mut harness = Harness::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("5.01")),
    );
    harness.teams = TeamsFake::new(TeamAnswer::Found(Box::new(team_row("team-1", "5", "5"))));
    let metadata = hook_metadata(key_auth(Some("10"), "1", Some("team-1")));
    assert!(key_or_team_over_budget(&metadata, &harness.deps()).await);
    let alerts = harness.alerts.team_alerts();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TeamAlertKind::TeamBudget);
    assert_eq!(alerts[0].spend, decimal("5.01"));
    assert_eq!(alerts[0].max_budget, Some(decimal("5")));
    assert_eq!(alerts[0].team_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn a_team_that_is_not_cached_is_not_over_budget() {
    let mut harness = Harness::new(SpendScript::Value(decimal("1")), SpendScript::Unreadable);
    harness.teams = TeamsFake::new(TeamAnswer::NotInCache);
    let metadata = hook_metadata(key_auth(Some("10"), "1", Some("team-1")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(harness.teams.lookups(), vec!["team-1".to_owned()]);
    assert_eq!(harness.spend.reads().len(), 1);
}

#[tokio::test]
async fn an_unreadable_key_spend_is_not_over_budget_and_skips_the_team() {
    let harness = Harness::new(SpendScript::Unreadable, SpendScript::Value(decimal("0")));
    let metadata = hook_metadata(key_auth(Some("10"), "1", Some("team-1")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(harness.spend.reads().len(), 1);
    assert!(harness.teams.lookups().is_empty());
}

#[tokio::test]
async fn an_unreadable_team_spend_is_not_over_budget() {
    let mut harness = Harness::new(SpendScript::Value(decimal("1")), SpendScript::Unreadable);
    harness.teams = TeamsFake::new(TeamAnswer::Found(Box::new(team_row("team-1", "5", "0"))));
    let metadata = hook_metadata(key_auth(Some("10"), "1", Some("team-1")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(harness.spend.reads().len(), 2);
}

#[tokio::test]
async fn a_key_without_a_team_skips_the_team_lookup() {
    let harness = Harness::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("0")),
    );
    let metadata = hook_metadata(key_auth(Some("10"), "1", None));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert!(harness.teams.lookups().is_empty());
}

#[tokio::test]
async fn an_empty_team_id_skips_the_team_lookup() {
    let harness = Harness::new(
        SpendScript::Value(decimal("1")),
        SpendScript::Value(decimal("0")),
    );
    let metadata = hook_metadata(key_auth(Some("10"), "1", Some("")));
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert!(harness.teams.lookups().is_empty());
}

#[tokio::test]
async fn a_future_temporary_increase_raises_the_advisory_limit() {
    let harness = Harness::new(
        SpendScript::Value(decimal("12")),
        SpendScript::Value(decimal("0")),
    );
    let mut auth = key_auth(Some("10"), "12", None);
    add_field(&mut auth, "temp_budget_increase", json("5"));
    add_field(
        &mut auth,
        "temp_budget_expiry",
        Json::Text("2099-01-01T00:00:00Z".to_owned()),
    );
    let metadata = hook_metadata(auth);
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(
        harness.spend.reads(),
        vec![SpendRead {
            counter_key: "spend:key:token-1".to_owned(),
            fallback_spend: decimal("12"),
            max_budget: Some(decimal("15")),
        }]
    );
}

#[tokio::test]
async fn an_expired_temporary_increase_is_not_applied() {
    let harness = Harness::new(
        SpendScript::Value(decimal("12")),
        SpendScript::Value(decimal("0")),
    );
    let mut auth = key_auth(Some("10"), "12", None);
    add_field(&mut auth, "temp_budget_increase", json("5"));
    add_field(
        &mut auth,
        "temp_budget_expiry",
        Json::Text("2000-01-01T00:00:00Z".to_owned()),
    );
    let metadata = hook_metadata(auth);
    assert!(key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert_eq!(
        harness.spend.reads(),
        vec![SpendRead {
            counter_key: "spend:key:token-1".to_owned(),
            fallback_spend: decimal("12"),
            max_budget: Some(decimal("10")),
        }]
    );
}

#[tokio::test]
async fn an_invalid_temporary_increase_is_not_over_budget() {
    let harness = Harness::new(
        SpendScript::Value(decimal("12")),
        SpendScript::Value(decimal("0")),
    );
    let mut auth = key_auth(Some("10"), "12", None);
    add_field(
        &mut auth,
        "temp_budget_increase",
        Json::Text("five".to_owned()),
    );
    add_field(
        &mut auth,
        "temp_budget_expiry",
        Json::Text("2099-01-01T00:00:00Z".to_owned()),
    );
    let metadata = hook_metadata(auth);
    assert!(!key_or_team_over_budget(&metadata, &harness.deps()).await);
    assert!(harness.spend.reads().is_empty());
}

#[tokio::test]
async fn the_budgetless_suppliers_answer_no_duration_and_no_membership() {
    assert_eq!(NoSources.seconds("30d", now()), None);
    let error = NoSources
        .team_membership("user-1", "team-1")
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "No db connected");
}

use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::Level;

use crate::auth_key_budgets::money::Money;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::alerts::{TenantAlertKind, TenantEventGroup};
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::project::{
    SoftOutcome, max_budget_check, max_budget_guard, soft_budget_check,
};

use super::support::{
    AlertFields, Recorder, ScriptedSpend, admission, capture_events, decimal, number, project,
    token,
};

#[tokio::test]
async fn project_absent_budget_and_nonfinite_skip() {
    let reader = ScriptedSpend::new(vec![number(100.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let absent = project(100.0_f64, None);
    max_budget_check(&absent, None, &admission)
        .await
        .expect("an absent budget table skips");
    let missing = project(100.0_f64, Some((None, Some(5.0_f64))));
    max_budget_check(&missing, None, &admission)
        .await
        .expect("an absent ceiling skips");
    max_budget_guard("proj-1", decimal("100"), Some(Money::Nan), None, &admission)
        .await
        .expect("a NaN ceiling skips before the read");
    max_budget_guard(
        "proj-1",
        decimal("100"),
        Some(Money::PositiveInfinity),
        None,
        &admission,
    )
    .await
    .expect("a positive infinity ceiling skips before the read");
    max_budget_guard(
        "proj-1",
        decimal("100"),
        Some(Money::NegativeInfinity),
        None,
        &admission,
    )
    .await
    .expect("a negative infinity ceiling skips before the read");
    assert!(reader.reads().is_empty());
    assert!(recorder.take().is_empty());
}

#[tokio::test]
async fn project_counter_key_and_fallback() {
    let reader = ScriptedSpend::new(vec![number(4.5_f64), number(10.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let allowed = project(4.5_f64, Some((Some(10.0_f64), None)));
    max_budget_check(&allowed, None, &admission)
        .await
        .expect("a spend below the ceiling passes");
    let denied = project(4.5_f64, Some((Some(10.0_f64), None)));
    assert!(matches!(
        max_budget_check(&denied, None, &admission).await,
        Err(TenantBudgetError::Denied(_))
    ));
    let reads = reader.reads();
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].counter_key, "spend:project:proj-1");
    assert_eq!(reads[0].fallback_spend, decimal("4.5"));
    assert_eq!(reads[0].max_budget, Some(decimal("10")));
    assert!(!reads[0].fallback_authoritative);
    assert_eq!(reads[1].counter_key, "spend:project:proj-1");
    assert_eq!(reads[1].fallback_spend, decimal("4.5"));
}

#[rstest]
#[case::below(9.99_f64, 10.0_f64, false)]
#[case::equality(10.0_f64, 10.0_f64, true)]
#[case::zero(0.0_f64, 0.0_f64, true)]
#[case::positive_spend_at_zero(0.01_f64, 0.0_f64, true)]
#[tokio::test]
async fn project_equality_and_zero_deny(
    #[case] spend: f64,
    #[case] ceiling: f64,
    #[case] denied: bool,
) {
    let reader = ScriptedSpend::new(vec![number(spend)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let outcome = max_budget_check(
        &project(spend, Some((Some(ceiling), None))),
        None,
        &admission,
    )
    .await;
    if denied {
        assert!(matches!(outcome, Err(TenantBudgetError::Denied(_))));
    } else {
        outcome.expect("a cent below the ceiling passes");
    }
    assert_eq!(reader.reads().len(), 1);
    assert!(recorder.take().is_empty());
}

#[tokio::test]
async fn project_alert_before_error() {
    let reader = ScriptedSpend::new(vec![number(11.0_f64)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let auth = token(AlertFields {
        token: "sk-project",
        org_id: Some("org-9"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        team_alias: Some("Team alias"),
        key_alias: Some("key-alias"),
    });
    let error = max_budget_check(
        &project(11.0_f64, Some((Some(10.0_f64), None))),
        Some(&auth),
        &admission,
    )
    .await
    .expect_err("a spend over the ceiling denies");
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TenantAlertKind::ProjectBudget);
    assert_eq!(alerts[0].event_group, TenantEventGroup::Project);
    assert_eq!(alerts[0].spend, decimal("11"));
    assert_eq!(alerts[0].max_budget, Some(decimal("10")));
    assert_eq!(alerts[0].soft_budget, None);
    assert_eq!(alerts[0].user_id.as_deref(), Some("user-1"));
    assert_eq!(alerts[0].team_id.as_deref(), Some("team-1"));
    assert_eq!(alerts[0].team_alias.as_deref(), Some("Team alias"));
    assert_eq!(alerts[0].organization_id.as_deref(), Some("org-9"));
    assert_eq!(
        alerts[0]
            .token
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("sk-project".to_owned())
    );
    assert!(matches!(error, TenantBudgetError::Denied(_)));
}

#[tokio::test]
async fn project_without_token_denies_without_alert() {
    let reader = ScriptedSpend::new(vec![number(11.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    assert!(matches!(
        max_budget_check(
            &project(11.0_f64, Some((Some(10.0_f64), None))),
            None,
            &admission
        )
        .await,
        Err(TenantBudgetError::Denied(_))
    ));
    assert!(recorder.take().is_empty());
    assert_eq!(reader.reads().len(), 1);
}

#[tokio::test]
async fn project_soft_row_spend_zero_boundary() {
    let recorder = Recorder::default();
    let auth = token(AlertFields {
        token: "sk-soft",
        org_id: None,
        user_id: Some("user-1"),
        team_id: None,
        team_alias: None,
        key_alias: None,
    });
    let zero = project(0.0_f64, Some((None, Some(0.0_f64))));
    assert_eq!(
        soft_budget_check(&zero, Some(&auth), Some(&recorder)),
        SoftOutcome::Alerted
    );
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].spend, decimal("0"));
    assert_eq!(alerts[0].soft_budget, Some(decimal("0")));
    let below = project(4.99_f64, Some((None, Some(5.0_f64))));
    assert_eq!(
        soft_budget_check(&below, Some(&auth), Some(&recorder)),
        SoftOutcome::NotCrossed
    );
    let equality = project(5.0_f64, Some((None, Some(5.0_f64))));
    assert_eq!(
        soft_budget_check(&equality, Some(&auth), Some(&recorder)),
        SoftOutcome::Alerted
    );
    let hard_only = project(9.0_f64, Some((Some(1.0_f64), None)));
    assert_eq!(
        soft_budget_check(&hard_only, Some(&auth), Some(&recorder)),
        SoftOutcome::NotCrossed
    );
    assert_eq!(recorder.take().len(), 1);
}

#[test]
fn project_soft_debug_without_token() {
    let (events, _guard) = capture_events();
    let crossed = project(6.0_f64, Some((None, Some(5.0_f64))));
    assert_eq!(
        soft_budget_check(&crossed, None, None),
        SoftOutcome::CrossedWithoutToken
    );
    let recorded = events.messages();
    assert!(
        recorded
            .iter()
            .any(|(level, message)| *level == Level::DEBUG
                && message == "Crossed Soft Budget for project"),
        "the soft check logs at debug: {recorded:?}"
    );
}

#[test]
fn project_soft_payload() {
    let recorder = Recorder::default();
    let auth = token(AlertFields {
        token: "sk-soft",
        org_id: Some("org-9"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        team_alias: Some("Team alias"),
        key_alias: None,
    });
    let row = project(7.0_f64, Some((None, Some(5.0_f64))));
    assert_eq!(
        soft_budget_check(&row, Some(&auth), Some(&recorder)),
        SoftOutcome::Alerted
    );
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TenantAlertKind::SoftBudget);
    assert_eq!(alerts[0].event_group, TenantEventGroup::Project);
    assert_eq!(alerts[0].spend, decimal("7"));
    assert_eq!(alerts[0].max_budget, None);
    assert_eq!(alerts[0].soft_budget, Some(decimal("5")));
    assert_eq!(alerts[0].organization_id.as_deref(), Some("org-9"));
}

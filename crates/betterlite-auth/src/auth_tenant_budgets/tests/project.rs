use secrecy::ExposeSecret as _;
use serde_json::json;
use tracing::Level;

use crate::auth_key_budgets::money::Money;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::alerts::{TenantAlertKind, TenantEventGroup};
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::project::{
    SoftOutcome, max_budget_check, max_budget_guard, soft_budget_check,
};

use super::support::{Recorder, ScriptedSpend, capture_events, decimal, number, project, token};

#[tokio::test]
async fn project_absent_budget_and_nonfinite_skip() {
    let reader = ScriptedSpend::new(vec![number(100.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let absent = project(100.0, None);
    max_budget_check(&absent, None, &admission)
        .await
        .expect("an absent budget table skips");
    let missing = project(100.0, Some((None, Some(5.0))));
    max_budget_check(&missing, None, &admission)
        .await
        .expect("an absent ceiling skips");
    for cap in [Money::Nan, Money::PositiveInfinity, Money::NegativeInfinity] {
        max_budget_guard("proj-1", decimal("100"), Some(cap), None, &admission)
            .await
            .expect("a nonfinite ceiling skips before the read");
    }
    assert!(reader.reads().is_empty());
    assert!(recorder.take().is_empty());
}

#[tokio::test]
async fn project_counter_key_and_fallback() {
    let reader = ScriptedSpend::new(vec![number(4.5), number(10.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let allowed = project(4.5, Some((Some(10.0), None)));
    max_budget_check(&allowed, None, &admission)
        .await
        .expect("a spend below the ceiling passes");
    let denied = project(4.5, Some((Some(10.0), None)));
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

#[tokio::test]
async fn project_equality_and_zero_deny() {
    let below = ScriptedSpend::new(vec![number(9.99)]);
    let below_recorder = Recorder::default();
    let below_admission = TenantAdmission {
        reader: &below,
        alerts: Some(&below_recorder),
    };
    max_budget_check(
        &project(9.99, Some((Some(10.0), None))),
        None,
        &below_admission,
    )
    .await
    .expect("a cent below the ceiling passes");

    let equality = ScriptedSpend::new(vec![number(10.0)]);
    let equality_recorder = Recorder::default();
    let equality_admission = TenantAdmission {
        reader: &equality,
        alerts: Some(&equality_recorder),
    };
    assert!(matches!(
        max_budget_check(
            &project(10.0, Some((Some(10.0), None))),
            None,
            &equality_admission
        )
        .await,
        Err(TenantBudgetError::Denied(_))
    ));

    let zero = ScriptedSpend::new(vec![number(0.0)]);
    let zero_recorder = Recorder::default();
    let zero_admission = TenantAdmission {
        reader: &zero,
        alerts: Some(&zero_recorder),
    };
    assert!(matches!(
        max_budget_check(
            &project(0.0, Some((Some(0.0), None))),
            None,
            &zero_admission
        )
        .await,
        Err(TenantBudgetError::Denied(_))
    ));
    assert!(matches!(
        max_budget_guard(
            "proj-1",
            decimal("0.01"),
            Some(Money::Finite(decimal("0"))),
            None,
            &zero_admission
        )
        .await,
        Err(TenantBudgetError::Denied(_))
    ));
}

#[tokio::test]
async fn project_alert_before_error() {
    let reader = ScriptedSpend::new(vec![number(11.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let auth = token(json!({
        "token": "sk-project",
        "user_id": "user-1",
        "team_id": "team-1",
        "team_alias": "Team alias",
        "org_id": "org-9",
        "key_alias": "key-alias",
    }));
    let error = max_budget_check(
        &project(11.0, Some((Some(10.0), None))),
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
    let reader = ScriptedSpend::new(vec![number(11.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    assert!(matches!(
        max_budget_check(&project(11.0, Some((Some(10.0), None))), None, &admission).await,
        Err(TenantBudgetError::Denied(_))
    ));
    assert!(recorder.take().is_empty());
    assert_eq!(reader.reads().len(), 1);
}

#[tokio::test]
async fn project_soft_row_spend_zero_boundary() {
    let recorder = Recorder::default();
    let auth = token(json!({"token": "sk-soft", "user_id": "user-1"}));
    let zero = project(0.0, Some((None, Some(0.0))));
    assert_eq!(
        soft_budget_check(&zero, Some(&auth), Some(&recorder)),
        SoftOutcome::Alerted
    );
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].spend, decimal("0"));
    assert_eq!(alerts[0].soft_budget, Some(decimal("0")));
    let below = project(4.99, Some((None, Some(5.0))));
    assert_eq!(
        soft_budget_check(&below, Some(&auth), Some(&recorder)),
        SoftOutcome::NotCrossed
    );
    let equality = project(5.0, Some((None, Some(5.0))));
    assert_eq!(
        soft_budget_check(&equality, Some(&auth), Some(&recorder)),
        SoftOutcome::Alerted
    );
    let hard_only = project(9.0, Some((Some(1.0), None)));
    assert_eq!(
        soft_budget_check(&hard_only, Some(&auth), Some(&recorder)),
        SoftOutcome::NotCrossed
    );
    assert_eq!(recorder.take().len(), 1);
}

#[test]
fn project_soft_debug_without_token() {
    let (events, _guard) = capture_events();
    let crossed = project(6.0, Some((None, Some(5.0))));
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
    let auth = token(json!({
        "token": "sk-soft",
        "user_id": "user-1",
        "team_id": "team-1",
        "team_alias": "Team alias",
        "org_id": "org-9",
    }));
    let row = project(7.0, Some((None, Some(5.0))));
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

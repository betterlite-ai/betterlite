use std::sync::Arc;

use secrecy::ExposeSecret as _;

use crate::auth_key_budgets::money::Money;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_org_lookup::ports::OrgCache;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::alerts::{TenantAlert, TenantAlertKind, TenantEventGroup};
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::organization::{max_budget_check, org_denies};
use crate::budget_snapshots::OrgBudgetSnapshot;

use super::support::{
    AlertFields, OrgWorld, Recorder, ScriptedSpend, TokenFields, admission, decimal, lookup,
    number, organization, team, token,
};

#[tokio::test]
async fn org_token_id_precedes_team_including_empty() {
    let world = Arc::new(OrgWorld::with_row(&organization("acme", 0.0_f64, None)));
    let lookup = lookup(&world);
    let reader = ScriptedSpend::new(Vec::new());
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let team = team(Some("team-org"));
    let mut token_org = token(TokenFields {
        token: "sk",
        org_id: Some("token-org"),
    });
    max_budget_check(Some(&mut token_org), Some(&team), Some(&lookup), &admission)
        .await
        .expect("an absent ceiling allows");
    let mut empty_org = token(TokenFields {
        token: "sk",
        org_id: Some(""),
    });
    max_budget_check(Some(&mut empty_org), Some(&team), Some(&lookup), &admission)
        .await
        .expect("an empty token organization still wins");
    let mut from_team = token(TokenFields {
        token: "sk",
        org_id: None,
    });
    max_budget_check(Some(&mut from_team), Some(&team), Some(&lookup), &admission)
        .await
        .expect("the team organization answers when the token has none");
    let mut nowhere = token(TokenFields {
        token: "sk",
        org_id: None,
    });
    max_budget_check(Some(&mut nowhere), None, Some(&lookup), &admission)
        .await
        .expect("no organization id skips");
    assert_eq!(
        world.lookups(),
        vec![
            ("token-org".to_owned(), true),
            (String::new(), true),
            ("team-org".to_owned(), true),
        ]
    );
    assert!(reader.reads().is_empty());
}

#[tokio::test]
async fn org_missing_context_skips() {
    let world = Arc::new(OrgWorld::with_row(&organization(
        "acme",
        0.0_f64,
        Some(10.0_f64),
    )));
    let lookup = lookup(&world);
    let reader = ScriptedSpend::new(vec![number(100.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let team = team(Some("team-org"));
    max_budget_check(None, Some(&team), Some(&lookup), &admission)
        .await
        .expect("a token is required");
    let mut auth = token(TokenFields {
        token: "sk",
        org_id: Some("token-org"),
    });
    max_budget_check(Some(&mut auth), Some(&team), None, &admission)
        .await
        .expect("a database is required");
    assert!(world.lookups().is_empty());
    assert!(reader.reads().is_empty());
}

#[tokio::test]
async fn org_lookup_error_and_missing_row_skip() {
    let missing = Arc::new(OrgWorld::default());
    let missing_lookup = lookup(&missing);
    let cache: Arc<dyn OrgCache> = Arc::<OrgWorld>::clone(&missing);
    let broken = OrgLookup {
        store: None,
        cache,
        in_memory_ttl: 60.0_f64,
    };
    let reader = ScriptedSpend::new(vec![number(100.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut absent_row = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    max_budget_check(
        Some(&mut absent_row),
        None,
        Some(&missing_lookup),
        &admission,
    )
    .await
    .expect("a missing row skips");
    let mut error = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    max_budget_check(Some(&mut error), None, Some(&broken), &admission)
        .await
        .expect("a lookup error skips");
    assert!(reader.reads().is_empty());
    assert!(absent_row.view.organization_alias.is_none());
    assert!(error.extra.org_budget_snapshot.is_none());
}

#[tokio::test]
async fn org_snapshot_before_missing_ceiling_return() {
    let world = Arc::new(OrgWorld::with_row(&organization("acme", 3.5_f64, None)));
    let lookup = lookup(&world);
    let reader = ScriptedSpend::new(Vec::new());
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut auth = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    max_budget_check(Some(&mut auth), None, Some(&lookup), &admission)
        .await
        .expect("a missing ceiling allows");
    assert_eq!(auth.view.organization_alias.as_deref(), Some("acme"));
    assert_eq!(
        auth.extra.org_budget_snapshot,
        Some(OrgBudgetSnapshot {
            spend: decimal("3.5"),
            max_budget: None,
        })
    );
    assert_eq!(world.writes(), vec!["org_id:org-1:with_budget".to_owned()]);
    assert!(reader.reads().is_empty());
}

#[tokio::test]
async fn org_counter_read_before_finite_test() {
    let world = Arc::new(OrgWorld::with_row(&organization(
        "acme",
        3.0_f64,
        Some(10.0_f64),
    )));
    let lookup = lookup(&world);
    let reader = ScriptedSpend::new(vec![number(5.0_f64)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut auth = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    max_budget_check(Some(&mut auth), None, Some(&lookup), &admission)
        .await
        .expect("a spend below the ceiling passes");
    let reads = reader.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:org:org-1");
    assert_eq!(reads[0].fallback_spend, decimal("3"));
    assert_eq!(reads[0].max_budget, Some(decimal("10")));
    assert!(!reads[0].fallback_authoritative);
    assert!(!org_denies(decimal("100"), Money::Nan));
    assert!(!org_denies(decimal("100"), Money::PositiveInfinity));
    assert!(!org_denies(decimal("100"), Money::NegativeInfinity));
    assert!(!org_denies(decimal("9.99"), Money::Finite(decimal("10"))));
    assert!(org_denies(decimal("10"), Money::Finite(decimal("10"))));
    assert!(org_denies(decimal("10.01"), Money::Finite(decimal("10"))));
}

// the identity fields the organization alert carries come from the token, and the amounts from the row and counter
fn assert_org_alert(alerts: &[TenantAlert]) {
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TenantAlertKind::OrganizationBudget);
    assert_eq!(alerts[0].event_group, TenantEventGroup::Organization);
    assert_eq!(alerts[0].spend, decimal("10"));
    assert_eq!(alerts[0].max_budget, Some(decimal("10")));
    assert_eq!(alerts[0].organization_id.as_deref(), Some("org-1"));
    assert_eq!(
        alerts[0]
            .token
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        Some("sk-org".to_owned())
    );
}

#[tokio::test]
async fn org_equality_alert_payload_and_error() {
    let world = Arc::new(OrgWorld::with_row(&organization(
        "acme",
        3.0_f64,
        Some(10.0_f64),
    )));
    let lookup = lookup(&world);
    let reader = ScriptedSpend::new(vec![number(10.0_f64)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let mut auth = token(AlertFields {
        token: "sk-org",
        org_id: Some("org-1"),
        user_id: Some("user-1"),
        team_id: Some("team-1"),
        team_alias: Some("Team alias"),
        key_alias: None,
    });
    let error = max_budget_check(Some(&mut auth), None, Some(&lookup), &admission)
        .await
        .expect_err("a spend at the ceiling denies");
    assert_org_alert(&recorder.take());
    let TenantBudgetError::Denied(denial) = error else {
        panic!("an organization denial is a BudgetExceededError");
    };
    assert_eq!(denial.status_code, 422);
    assert_eq!(denial.current_cost, decimal("10"));
    assert_eq!(denial.max_budget, decimal("10"));
    assert_eq!(denial.entity_type, "organization");
    assert_eq!(
        denial
            .entity_id
            .as_ref()
            .map(|entity| entity.expose_secret().to_owned()),
        Some("org-1".to_owned())
    );
    assert!(
        denial
            .message
            .starts_with("Budget has been exceeded! Organization=org-1 Current cost:")
    );
    assert!(denial.message.contains(", Max budget:"));
}

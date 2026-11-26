use std::sync::Arc;

use crate::auth_tenant_budgets::TenantRequest;
use crate::auth_tenant_budgets::admit;
use crate::auth_tenant_budgets::alerts::TenantAlertKind;
use crate::auth_tenant_budgets::error::TenantBudgetError;
use secrecy::ExposeSecret as _;

use super::support::{
    OrgWorld, Recorder, ScriptedSpend, TagWorld, TokenFields, admission, group_with_budget, lookup,
    number, organization, project, tag_with_budget, tenant_sources, token,
};

// common_checks raises 'Project=<id>', 'Organization=<id>', 'Tag=<name>' or 'Model access group=<name>'
fn denied(error: TenantBudgetError) -> (String, String) {
    let TenantBudgetError::Denied(denial) = error else {
        panic!("the request must be denied by a budget");
    };
    let entity_id = denial
        .entity_id
        .expect("a tenant denial names its entity")
        .expose_secret()
        .to_owned();
    (denial.entity_type, entity_id)
}

fn counter_keys(reader: &ScriptedSpend) -> Vec<String> {
    reader
        .reads()
        .into_iter()
        .map(|read| read.counter_key)
        .collect()
}

// all three settled scopes price and read the counter, and the organization denial is the one reported
fn assert_settled_org_denial(
    reader: &ScriptedSpend,
    world: &TagWorld,
    org: &OrgWorld,
    recorder: &Recorder,
) {
    assert_eq!(
        counter_keys(reader),
        vec![
            "spend:org:org-1",
            "spend:tag:t",
            "spend:model_access_group:g"
        ]
    );
    assert_eq!(world.batches(), (1, 1));
    assert_eq!(org.lookups(), vec![("org-1".to_owned(), true)]);
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TenantAlertKind::OrganizationBudget);
}

#[tokio::test]
async fn project_denial_stops_the_settled_scopes() {
    let world = TagWorld::new(
        vec![tag_with_budget("t", 100.0_f64, Some(1.0_f64))],
        vec![group_with_budget("g", 100.0_f64, Some(1.0_f64))],
    );
    let reader = ScriptedSpend::new(vec![number(11.0_f64)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let mut token = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    let projected = project(10.0_f64, Some((Some(10.0_f64), None)));
    let names = ["t".to_owned()];
    let groups = ["g".to_owned()];
    let sources = tenant_sources(&world, true);
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: Some(&projected),
        tags: &names,
        matched_model_access_groups: &groups,
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the project is over its maximum");
    assert_eq!(denied(error), ("project".to_owned(), "proj-1".to_owned()));
    assert_eq!(counter_keys(&reader), vec!["spend:project:proj-1"]);
    assert_eq!(world.batches(), (0, 0));
    let alerts = recorder.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, TenantAlertKind::ProjectBudget);
}

#[tokio::test]
async fn organization_denial_wins_the_settled_scopes() {
    let world = TagWorld::new(
        vec![tag_with_budget("t", 9.0_f64, Some(9.0_f64))],
        vec![group_with_budget("g", 9.0_f64, Some(9.0_f64))],
    );
    let org = Arc::new(OrgWorld::with_row(&organization(
        "acme",
        3.0_f64,
        Some(10.0_f64),
    )));
    let lookup = lookup(&org);
    let reader = ScriptedSpend::new(vec![number(10.0_f64), number(10.0_f64), number(10.0_f64)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let mut token = token(TokenFields {
        token: "sk",
        org_id: Some("org-1"),
    });
    let names = ["t".to_owned()];
    let groups = ["g".to_owned()];
    let mut sources = tenant_sources(&world, true);
    sources.organizations = Some(&lookup);
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &names,
        matched_model_access_groups: &groups,
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("all three scopes are over their maximum");
    assert_eq!(
        denied(error),
        ("organization".to_owned(), "org-1".to_owned())
    );
    assert_settled_org_denial(&reader, &world, &org, &recorder);
}

#[tokio::test]
async fn tag_denial_beats_the_group_denial() {
    let world = TagWorld::new(
        vec![tag_with_budget("t", 9.0_f64, Some(9.0_f64))],
        vec![group_with_budget("g", 0.0_f64, Some(1.0_f64))],
    );
    let reader = ScriptedSpend::new(vec![number(10.0_f64), number(2.0_f64)]);
    let recorder = Recorder::default();
    let admission = admission(&reader, &recorder);
    let mut token = token(TokenFields {
        token: "sk",
        org_id: None,
    });
    let names = ["t".to_owned()];
    let groups = ["g".to_owned()];
    let sources = tenant_sources(&world, true);
    let mut request = TenantRequest {
        token: Some(&mut token),
        team: None,
        project: None,
        tags: &names,
        matched_model_access_groups: &groups,
    };
    let error = admit(&mut request, &sources, &admission)
        .await
        .expect_err("the tag and the group are both over budget");
    assert_eq!(denied(error), ("tag".to_owned(), "t".to_owned()));
    assert_eq!(
        counter_keys(&reader),
        vec!["spend:tag:t", "spend:model_access_group:g"]
    );
    assert_eq!(world.batches(), (1, 1));
    assert!(recorder.take().is_empty());
}

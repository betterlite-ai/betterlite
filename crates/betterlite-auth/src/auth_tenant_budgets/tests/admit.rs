use std::sync::Arc;

use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_tenant_budgets::TenantAdmission;
use crate::auth_tenant_budgets::TenantRequest;
use crate::auth_tenant_budgets::admit;
use crate::auth_tenant_budgets::alerts::TenantAlertKind;
use crate::auth_tenant_budgets::error::TenantBudgetError;

use super::support::{
    OrgWorld, Recorder, ScriptedSpend, TagWorld, group, lookup, number, organization, project, tag,
    tenant_sources, token,
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

#[tokio::test]
async fn project_denial_stops_the_settled_scopes() {
    let world = TagWorld::new(
        vec![tag("t", 100.0, Some(Some(1.0)))],
        vec![group("g", 100.0, Some(Some(1.0)))],
    );
    let reader = ScriptedSpend::new(vec![number(11.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut token = token(json!({"token": "sk", "org_id": "org-1"}));
    let projected = project(10.0, Some((Some(10.0), None)));
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
        vec![tag("t", 9.0, Some(Some(9.0)))],
        vec![group("g", 9.0, Some(Some(9.0)))],
    );
    let org = Arc::new(OrgWorld::with_row(&organization("acme", 3.0, Some(10.0))));
    let lookup = lookup(&org);
    let reader = ScriptedSpend::new(vec![number(10.0), number(10.0), number(10.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut token = token(json!({"token": "sk", "org_id": "org-1"}));
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
    assert_eq!(
        counter_keys(&reader),
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
async fn tag_denial_beats_the_group_denial() {
    let world = TagWorld::new(
        vec![tag("t", 9.0, Some(Some(9.0)))],
        vec![group("g", 0.0, Some(Some(1.0)))],
    );
    let reader = ScriptedSpend::new(vec![number(10.0), number(2.0)]);
    let recorder = Recorder::default();
    let admission = TenantAdmission {
        reader: &reader,
        alerts: Some(&recorder),
    };
    let mut token = token(json!({"token": "sk"}));
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

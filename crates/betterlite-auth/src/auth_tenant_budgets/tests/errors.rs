use std::sync::Arc;

use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::auth_tenant_budgets::access_groups::access_group_max_budget_check as group_check;
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::organization::max_budget_check as organization_check;
use crate::auth_tenant_budgets::project::max_budget_check as project_check;
use crate::auth_tenant_budgets::tags::max_budget_check as tag_check;

use super::support::{
    OrgWorld, Recorder, ScriptedSpend, TagWorld, admission, decimal, group_with_budget, lookup,
    number, organization as organization_row, project as project_row, tag_with_budget, team,
    tenant_sources, token,
};

// every tenant denial is a BudgetExceededError: 422, the entity type and id, both amounts, and the exact message
fn assert_denial(
    error: TenantBudgetError,
    entity_type: &str,
    entity_id: &str,
    current_cost: &str,
    max_budget: &str,
) {
    let label = match entity_type {
        "project" => "Project",
        "organization" => "Organization",
        "tag" => "Tag",
        "model_access_group" => "Model access group",
        other => panic!("no tenant entity is named {other}"),
    };
    let TenantBudgetError::Denied(denial) = error else {
        panic!("a tenant budget denial is a BudgetExceededError");
    };
    assert_eq!(denial.status_code, 422);
    assert_eq!(denial.entity_type, entity_type);
    assert_eq!(denial.current_cost, decimal(current_cost));
    assert_eq!(denial.max_budget, decimal(max_budget));
    assert_eq!(
        denial
            .entity_id
            .as_ref()
            .map(|entity| entity.expose_secret().to_owned()),
        Some(entity_id.to_owned())
    );
    assert_eq!(
        denial.message,
        format!(
            "Budget has been exceeded! {label}={entity_id} Current cost: {}, Max budget: {}",
            denial.current_cost, denial.max_budget
        )
    );
}

async fn project_denial(recorder: &Recorder) {
    let reader = ScriptedSpend::new(vec![number(11.0)]);
    let admission = admission(&reader, recorder);
    let error = project_check(
        &project_row(11.0, Some((Some(10.0), None))),
        None,
        &admission,
    )
    .await
    .expect_err("the project spend is over its ceiling");
    assert_denial(error, "project", "proj-1", "11", "10");
}

async fn organization_denial(recorder: &Recorder) {
    let world = Arc::new(OrgWorld::with_row(&organization_row(
        "acme",
        3.0,
        Some(10.0),
    )));
    let org_lookup = lookup(&world);
    let reader = ScriptedSpend::new(vec![number(10.0)]);
    let admission = admission(&reader, recorder);
    let mut auth = token(json!({"token": "sk-org", "org_id": "org-1"}));
    let error = organization_check(
        Some(&mut auth),
        Some(&team(None)),
        Some(&org_lookup),
        &admission,
    )
    .await
    .expect_err("the organization spend equals its ceiling");
    assert_denial(error, "organization", "org-1", "10", "10");
}

async fn tag_denial(recorder: &Recorder) {
    let world = TagWorld::new(vec![tag_with_budget("t", 0.0, Some(10.0))], vec![]);
    let sources = tenant_sources(&world, true);
    let reader = ScriptedSpend::new(vec![number(10.01)]);
    let admission = admission(&reader, recorder);
    let error = tag_check(&["t".to_owned()], &sources, &admission)
        .await
        .expect_err("the tag spend is over its ceiling");
    assert_denial(error, "tag", "t", "10.01", "10");
}

async fn group_denial(recorder: &Recorder) {
    let world = TagWorld::new(vec![], vec![group_with_budget("g", 10.0, Some(10.0))]);
    let sources = tenant_sources(&world, true);
    let reader = ScriptedSpend::new(vec![number(10.0)]);
    let admission = admission(&reader, recorder);
    let error = group_check(&["g".to_owned()], &sources, &admission)
        .await
        .expect_err("the group spend equals its ceiling");
    assert_denial(error, "model_access_group", "g", "10", "10");
}

#[tokio::test]
async fn all_budget_error_fields_and_default_422() {
    let recorder = Recorder::default();
    project_denial(&recorder).await;
    organization_denial(&recorder).await;
    tag_denial(&recorder).await;
    group_denial(&recorder).await;
}

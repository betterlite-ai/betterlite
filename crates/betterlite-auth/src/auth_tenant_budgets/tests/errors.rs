use std::future::Future;
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

async fn denial_of<F>(future: F, entity_type: &str, entity_id: &str, spend: &str, ceiling: &str)
where
    F: Future<Output = Result<(), TenantBudgetError>>,
{
    let error = future.await.expect_err("the spend is over the ceiling");
    assert_denial(error, entity_type, entity_id, spend, ceiling);
}

#[tokio::test]
async fn all_budget_error_fields_and_default_422() {
    let recorder = Recorder::default();
    let project_reader = ScriptedSpend::new(vec![number(11.0)]);
    let project_admission = admission(&project_reader, &recorder);
    let project_value = project_row(11.0, Some((Some(10.0), None)));
    let project_future = project_check(&project_value, None, &project_admission);
    denial_of(project_future, "project", "proj-1", "11", "10").await;

    let org_row = organization_row("acme", 3.0, Some(10.0));
    let org_world = Arc::new(OrgWorld::with_row(&org_row));
    let org_lookup = lookup(&org_world);
    let org_reader = ScriptedSpend::new(vec![number(10.0)]);
    let org_admission = admission(&org_reader, &recorder);
    let mut org_token = token(json!({"token": "sk-org", "org_id": "org-1"}));
    let org_team = team(None);
    let org_future = organization_check(
        Some(&mut org_token),
        Some(&org_team),
        Some(&org_lookup),
        &org_admission,
    );
    denial_of(org_future, "organization", "org-1", "10", "10").await;

    let tag_world = TagWorld::new(vec![tag_with_budget("t", 0.0, Some(10.0))], vec![]);
    let tag_sources = tenant_sources(&tag_world, true);
    let tag_names = ["t".to_owned()];
    let tag_reader = ScriptedSpend::new(vec![number(10.01)]);
    let tag_admission = admission(&tag_reader, &recorder);
    let tag_future = tag_check(&tag_names, &tag_sources, &tag_admission);
    denial_of(tag_future, "tag", "t", "10.01", "10").await;

    let group_world = TagWorld::new(vec![], vec![group_with_budget("g", 10.0, Some(10.0))]);
    let group_sources = tenant_sources(&group_world, true);
    let group_names = ["g".to_owned()];
    let group_reader = ScriptedSpend::new(vec![number(10.0)]);
    let group_admission = admission(&group_reader, &recorder);
    let group_future = group_check(&group_names, &group_sources, &group_admission);
    denial_of(group_future, "model_access_group", "g", "10", "10").await;
}

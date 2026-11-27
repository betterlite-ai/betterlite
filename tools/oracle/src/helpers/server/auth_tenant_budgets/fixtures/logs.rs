use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_tenant_budgets::project::SoftOutcome;
use rust_decimal::prelude::ToPrimitive as _;

// verbose_proxy_logger.debug's text in _project_soft_budget_check, with Python's float repr of the row's amounts
pub(crate) fn soft_logs(outcome: SoftOutcome, project: &CachedProject) -> Vec<String> {
    let row = &project.project;
    let Some(soft_budget) = row
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.soft_budget)
    else {
        return Vec::new();
    };
    match outcome {
        SoftOutcome::NotCrossed => Vec::new(),
        SoftOutcome::CrossedWithoutToken | SoftOutcome::Alerted => vec![format!(
            "Crossed Soft Budget for project {}, spend {:?}, soft_budget {:?}",
            row.project_id,
            row.spend.to_f64().unwrap_or(f64::NAN),
            soft_budget.to_f64().unwrap_or(f64::NAN),
        )],
    }
}

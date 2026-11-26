use rust_decimal::Decimal;
use tracing::debug;

use crate::auth_cache::keys::CacheKey;
use crate::auth_key_budgets::money::Money;
use crate::auth_prefetch::rows::CachedProject;
use crate::current_spend::{SpendQuery, WindowDescriptors};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::TenantAdmission;
use super::alerts::{self, TenantAlertFields, TenantAlertKind, TenantAlertSink, TenantEventGroup};
use super::denial::{Entity, denial};
use super::error::TenantBudgetError;

// no tenant check reads a budget window
pub(crate) const NO_WINDOW: WindowDescriptors<'static> = WindowDescriptors {
    entity_type: None,
    entity_id: None,
    duration: None,
    start: None,
};

// what the project soft check did; it never denies, so a caller reads it to see which log or alert happened
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftOutcome {
    NotCrossed,
    CrossedWithoutToken,
    Alerted,
}

// _project_max_budget_check: a missing or nonfinite ceiling skips before the read, and a token alerts before the denial
pub async fn max_budget_check(
    project: &CachedProject,
    token: Option<&UserAPIKeyAuth>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    let row = &project.project;
    let cap = row
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.max_budget);
    max_budget_guard(
        &row.project_id,
        row.spend,
        cap.map(Money::Finite),
        token,
        admission,
    )
    .await
}

// LiteLLM tests math.isfinite before reading, so a nonfinite ceiling costs no counter read and never denies
pub(crate) async fn max_budget_guard(
    project_id: &str,
    fallback_spend: Decimal,
    cap: Option<Money>,
    token: Option<&UserAPIKeyAuth>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    let Some(ceiling) = cap.and_then(Money::finite_value) else {
        return Ok(());
    };
    let counter_key = CacheKey::ProjectSpend(project_id).render();
    let query = SpendQuery {
        counter_key: &counter_key,
        fallback_spend,
        max_budget: Some(ceiling),
        window: NO_WINDOW,
        fallback_authoritative: false,
    };
    let spend = admission.reader.read_spend(&query).await?;
    if spend < ceiling {
        return Ok(());
    }
    if let Some(token) = token {
        alerts::schedule(
            admission.alerts,
            alerts::identity_alert(
                token,
                TenantAlertFields {
                    kind: TenantAlertKind::ProjectBudget,
                    event_group: TenantEventGroup::Project,
                    spend,
                    max_budget: Some(ceiling),
                    soft_budget: None,
                    organization_id: token.view.token.org_id.clone(),
                },
            ),
        );
    }
    Err(TenantBudgetError::Denied(denial(
        Entity::Project(project_id),
        spend,
        ceiling,
    )))
}

// _project_soft_budget_check: the row's spend at or above soft_budget logs, even at zero, and only a token alerts
#[must_use]
pub fn soft_budget_check(
    project: &CachedProject,
    token: Option<&UserAPIKeyAuth>,
    alerts: Option<&dyn TenantAlertSink>,
) -> SoftOutcome {
    let row = &project.project;
    let Some(soft_budget) = row
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.soft_budget)
    else {
        return SoftOutcome::NotCrossed;
    };
    if row.spend < soft_budget {
        return SoftOutcome::NotCrossed;
    }
    debug!(
        project_id = %row.project_id,
        spend = %row.spend,
        soft_budget = %soft_budget,
        "Crossed Soft Budget for project"
    );
    let Some(token) = token else {
        return SoftOutcome::CrossedWithoutToken;
    };
    alerts::schedule(
        alerts,
        alerts::identity_alert(
            token,
            TenantAlertFields {
                kind: TenantAlertKind::SoftBudget,
                event_group: TenantEventGroup::Project,
                spend: row.spend,
                max_budget: None,
                soft_budget: Some(soft_budget),
                organization_id: token.view.token.org_id.clone(),
            },
        ),
    );
    SoftOutcome::Alerted
}

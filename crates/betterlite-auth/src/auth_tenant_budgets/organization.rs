use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;

use crate::auth_key_budgets::money::Money;
use crate::auth_org_lookup::organization::{OrgId, OrgLookup, get_org_object};
use crate::budget_primitives::carried::carry_organization;
use crate::current_spend::SpendQuery;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::TenantAdmission;
use super::alerts::{self, TenantAlertFields, TenantAlertKind, TenantEventGroup};
use super::denial::{Entity, denial};
use super::error::TenantBudgetError;
use super::project::NO_WINDOW;

// a lookup failure or a missing row skips, the budget state is carried, and a finite ceiling denies at equality after the alert; the failure is swallowed on purpose
pub async fn max_budget_check(
    token: Option<&mut UserAPIKeyAuth>,
    team: Option<&LiteLLMTeamTable>,
    lookup: Option<&OrgLookup>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    let Some(token) = token else {
        return Ok(());
    };
    let Some(lookup) = lookup else {
        return Ok(());
    };
    let Some(org_id) = organization_id(token, team) else {
        return Ok(());
    };
    let organization = match get_org_object(lookup, OrgId::Text(org_id.clone()), true).await {
        Ok(Some(organization)) => organization,
        Ok(None) | Err(_) => return Ok(()),
    };
    carry_organization(token, &organization);
    let Some(ceiling) = organization
        .litellm_budget_table
        .as_ref()
        .and_then(|table| table.max_budget)
    else {
        return Ok(());
    };
    enforce_ceiling(token, &org_id, organization.spend, ceiling, admission).await
}

// the armed counter and the alert-before-error once the carried organization has a finite-shaped ceiling
async fn enforce_ceiling(
    token: &UserAPIKeyAuth,
    org_id: &str,
    fallback_spend: Decimal,
    ceiling: Decimal,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    let counter_key = format!("spend:org:{org_id}");
    let query = SpendQuery {
        counter_key: &counter_key,
        fallback_spend,
        max_budget: Some(ceiling),
        window: NO_WINDOW,
        fallback_authoritative: false,
    };
    let spend = admission.reader.read_spend(&query).await?;
    if !org_denies(spend, Money::Finite(ceiling)) {
        return Ok(());
    }
    alerts::schedule(
        admission.alerts,
        alerts::identity_alert(
            token,
            TenantAlertFields {
                kind: TenantAlertKind::OrganizationBudget,
                event_group: TenantEventGroup::Organization,
                spend,
                max_budget: Some(ceiling),
                soft_budget: None,
                organization_id: Some(org_id.to_owned()),
            },
        ),
    );
    Err(TenantBudgetError::Denied(denial(
        Entity::Organization(org_id),
        spend,
        ceiling,
    )))
}

// a nonnull token organization id wins, including an empty string; the team's id answers only when the token has none
fn organization_id(token: &UserAPIKeyAuth, team: Option<&LiteLLMTeamTable>) -> Option<String> {
    if let Some(org_id) = token.view.token.org_id.clone() {
        return Some(org_id);
    }
    team.and_then(|row| row.fields.base.organization_id.clone())
}

// Python's `math.isfinite(ceiling) and spend >= ceiling`; a nonfinite ceiling reads the counter and allows
pub(crate) fn org_denies(spend: Decimal, ceiling: Money) -> bool {
    match ceiling {
        Money::Finite(limit) => spend >= limit,
        Money::Nan | Money::PositiveInfinity | Money::NegativeInfinity => false,
    }
}

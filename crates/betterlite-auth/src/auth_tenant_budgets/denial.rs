use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};
use crate::auth_key_budgets::money::Money;
use crate::current_spend::SpendQuery;

use super::TenantAdmission;
use super::error::TenantBudgetError;
use super::project::NO_WINDOW;

// the entity a tenant budget denial names, with the value Litellm_EntityType and the message's label use
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entity<'entity> {
    Project(&'entity str),
    Organization(&'entity str),
    Tag(&'entity str),
    ModelAccessGroup(&'entity str),
}

// BudgetExceededError's caller-visible facts; B757 renders its status, type, param and code
pub(crate) fn denial(entity: Entity<'_>, spend: Decimal, max_budget: Decimal) -> Denial {
    let (entity_type, label, entity_id) = match entity {
        Entity::Project(entity_id) => ("project", "Project", entity_id),
        Entity::Organization(entity_id) => ("organization", "Organization", entity_id),
        Entity::Tag(entity_id) => ("tag", "Tag", entity_id),
        Entity::ModelAccessGroup(entity_id) => {
            ("model_access_group", "Model access group", entity_id)
        }
    };
    Denial {
        current_cost: spend,
        max_budget,
        entity_type: entity_type.to_owned(),
        entity_id: Some(SecretString::from(entity_id.to_owned())),
        message: format!(
            "Budget has been exceeded! {label}={entity_id} Current cost: {spend}, Max budget: {max_budget}"
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }
}

// one row a tag or access-group check prices: its counter, the spend the row carries and the comparison it applies
pub(crate) struct PricedRow<'row> {
    pub(crate) entity: Entity<'row>,
    pub(crate) counter_key: String,
    pub(crate) fallback_spend: Decimal,
    pub(crate) ceiling: Decimal,
    pub(crate) denies: fn(Decimal, Money) -> bool,
}

// the first priced row whose authoritative counter denies stops the batch, which is what both batch checks do
pub(crate) async fn read_and_deny(
    rows: Vec<PricedRow<'_>>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    for row in rows {
        let query = SpendQuery {
            counter_key: &row.counter_key,
            fallback_spend: row.fallback_spend,
            max_budget: Some(row.ceiling),
            window: NO_WINDOW,
            fallback_authoritative: true,
        };
        let spend = admission.reader.read_spend(&query).await?;
        if !(row.denies)(spend, Money::Finite(row.ceiling)) {
            continue;
        }
        let denied = denial(row.entity, spend, row.ceiling);
        return Err(TenantBudgetError::Denied(denied));
    }
    Ok(())
}

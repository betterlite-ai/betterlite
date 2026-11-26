use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};

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

use std::collections::BTreeMap;

use betterlite_auth::model_budget_values::budget_config::BudgetRuntime;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_auth::model_budget_values::resolution::resolve_model_budget;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::proxy_request_contracts::decode;
use crate::team_limits::defaults::Caller;
use crate::team_limits::field_supplied;
use crate::team_request_contracts::update::UpdateTeamRequest;
use crate::team_request_contracts::values::BudgetConfig;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelBudgetUpdateError {
    #[error(
        "Only a proxy admin can remove a team's model_max_budget for '{model}'. Current max_budget={max_budget}."
    )]
    RemoveDenied { model: String, max_budget: Decimal },
    #[error(
        "Only a proxy admin can raise a team's model_max_budget for '{model}' or change its budget_duration. Current max_budget={max_budget} per {duration} (entry '{entry}'), requested={requested} per {requested_duration}."
    )]
    RaiseDenied {
        model: String,
        max_budget: Decimal,
        duration: String,
        entry: String,
        requested: String,
        requested_duration: String,
    },
}

// BudgetConfig.model_validate failure reads as an absent cap, so a malformed entry imposes no ceiling
#[must_use]
pub fn existing_model_cap(raw: &JsonData) -> Option<BudgetConfig> {
    let JsonData::Object(fields) = raw else {
        return None;
    };
    decode::<BudgetConfig>(fields).ok()
}

pub fn model_budget_authority(
    request: &UpdateTeamRequest,
    caller: &Caller,
    existing_model_max_budget: Option<&BTreeMap<String, JsonData>>,
    catalog: &dyn BedrockCatalog,
    runtime: BudgetRuntime<'_>,
) -> Result<(), ModelBudgetUpdateError> {
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let Some(existing) = existing_model_max_budget else {
        return Ok(());
    };
    if existing.is_empty() || !field_supplied(request, "model_max_budget") {
        return Ok(());
    }
    let requested = request.fields.model_max_budget.as_ref();
    for (model, raw_existing) in existing {
        let Some(cap) = existing_model_cap(raw_existing) else {
            continue;
        };
        let Some(max_budget) = cap.max_budget else {
            continue;
        };
        if requested.is_some_and(|proposed| proposed.contains_key(model)) {
            continue;
        }
        return Err(ModelBudgetUpdateError::RemoveDenied {
            model: model.clone(),
            max_budget: max_budget.0,
        });
    }
    let Some(proposed_entries) = requested else {
        return Ok(());
    };
    for (model, proposed) in proposed_entries {
        check_governing_cap(model, proposed, existing, catalog, runtime)?;
    }
    Ok(())
}

fn check_governing_cap(
    model: &str,
    proposed: &BudgetConfig,
    existing: &BTreeMap<String, JsonData>,
    catalog: &dyn BedrockCatalog,
    runtime: BudgetRuntime<'_>,
) -> Result<(), ModelBudgetUpdateError> {
    let Some(governing) = resolve_model_budget(model, existing, catalog, runtime) else {
        return Ok(());
    };
    let cap = governing.budget_config;
    let Some(max_budget) = cap.max_budget else {
        return Ok(());
    };
    let raised = match proposed.max_budget {
        None => true,
        Some(amount) => amount.0 > max_budget,
    };
    let rewound = proposed.budget_duration.as_deref() != Some(cap.budget_duration.as_str());
    if !raised && !rewound {
        return Ok(());
    }
    Err(ModelBudgetUpdateError::RaiseDenied {
        model: model.to_owned(),
        max_budget,
        duration: cap.budget_duration,
        entry: governing.budget_model,
        // LiteLLM prints the requested values with Python's None for an absent one
        requested: proposed
            .max_budget
            .map_or_else(|| "None".to_owned(), |value| value.0.to_string()),
        requested_duration: proposed
            .budget_duration
            .as_deref()
            .map_or_else(|| "None".to_owned(), str::to_owned),
    })
}

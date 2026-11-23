use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_config::sections::auth_user_lookup::DefaultUserParams;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::analytics::python::float_text;
use betterlite_models::key_digest::hashing::hash_token;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use thiserror::Error;
use tracing::info;

use crate::startup_checks::error::StartupCheckError;

#[derive(Debug, Error)]
pub enum TeamEntryProblem {
    #[error("litellm_settings.default_internal_user_params.teams[{index}] is not an object")]
    NotAnObject { index: usize },
    #[error(
        "litellm_settings.default_internal_user_params.teams[{index}].team_id must be a string"
    )]
    TeamIdNotText { index: usize },
    #[error(
        "litellm_settings.default_internal_user_params.teams[{index}].max_budget_in_team must be a number or null"
    )]
    BudgetNotANumber { index: usize },
}

// update_default_team_member_budget: the admin-API team update, run as the proxy admin
#[async_trait]
pub trait TeamMemberBudgetUpdater: Send + Sync {
    async fn update_member_budget(
        &self,
        team_id: &str,
        max_budget_in_team: Option<Decimal>,
        admin_token: &SecretString,
    ) -> Result<(), PortError>;
}

struct TeamRequest {
    team_id: String,
    max_budget_in_team: Option<Decimal>,
}

pub async fn update_default_team_budgets(
    params: Option<&DefaultUserParams>,
    master_key: &SecretString,
    updater: &dyn TeamMemberBudgetUpdater,
) -> Result<(), StartupCheckError> {
    let Some(defaults) = params else {
        return Ok(());
    };
    let Some(teams) = defaults.values.get("teams") else {
        return Ok(());
    };
    match teams {
        JsonData::Array(entries) => apply_team_budgets(entries, master_key, updater).await,
        JsonData::Null => Ok(()),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Object(_) => {
            Err(StartupCheckError::TeamsNotAList)
        }
    }
}

async fn apply_team_budgets(
    teams: &[JsonData],
    master_key: &SecretString,
    updater: &dyn TeamMemberBudgetUpdater,
) -> Result<(), StartupCheckError> {
    let requests = team_requests(teams)?;
    if requests.is_empty() {
        return Ok(());
    }
    let admin_token = SecretString::from(hash_token(master_key.expose_secret()));
    for request in requests {
        let outcome = updater
            .update_member_budget(&request.team_id, request.max_budget_in_team, &admin_token)
            .await;
        if let Err(error) = outcome {
            info!(
                "Error updating team {} with team member budget {} with error: {error}, skipping..",
                request.team_id,
                budget_text(request.max_budget_in_team)
            );
        }
    }
    Ok(())
}

fn team_requests(teams: &[JsonData]) -> Result<Vec<TeamRequest>, TeamEntryProblem> {
    let mut requests = Vec::with_capacity(teams.len());
    for (index, team) in teams.iter().enumerate() {
        let JsonData::Object(fields) = team else {
            return Err(TeamEntryProblem::NotAnObject { index });
        };
        let team_id = match fields.get("team_id") {
            Some(JsonData::String(text)) => text.clone(),
            Some(
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::Array(_)
                | JsonData::Object(_),
            )
            | None => return Err(TeamEntryProblem::TeamIdNotText { index }),
        };
        let max_budget_in_team = match fields.get("max_budget_in_team") {
            None | Some(JsonData::Null) => None,
            Some(JsonData::Number(number)) => Some(
                number_to_decimal(number).ok_or(TeamEntryProblem::BudgetNotANumber { index })?,
            ),
            Some(
                JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_),
            ) => {
                return Err(TeamEntryProblem::BudgetNotANumber { index });
            }
        };
        requests.push(TeamRequest {
            team_id,
            max_budget_in_team,
        });
    }
    Ok(requests)
}

fn number_to_decimal(number: &Number) -> Option<Decimal> {
    Decimal::from_str(&number.to_string())
        .ok()
        .or_else(|| number.as_f64().and_then(Decimal::from_f64))
}

// NewUserRequestTeam.max_budget_in_team is a Python float, so the log line carries its repr
fn budget_text(budget: Option<Decimal>) -> String {
    budget.map_or_else(
        || "None".to_owned(),
        |value| value.to_f64().map_or_else(|| value.to_string(), float_text),
    )
}

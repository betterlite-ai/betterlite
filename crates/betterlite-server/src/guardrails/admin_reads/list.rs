use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_config::sections::guardrail_params::routes::{
    GuardrailInfoResponse, ListGuardrailsResponse,
};
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;

use super::caller::{authenticated, has_admin_view, team_ids};
use super::error::ReadError;
use super::masking::mask_sensitive_values;
use super::projection::{base_params, config_dump, stored_dump, validated};
use super::state::{ConfigGuardrail, ReadState};
use crate::auth::auth_identity::AuthContext;

// the raw guardrails: list of the loaded config file, never the database
pub(super) async fn list_guardrails<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
) -> Result<Json<ListGuardrailsResponse>, ReadError>
where
    R: GuardrailRows,
{
    authenticated(caller)?;
    let entries = state
        .config
        .guardrails()
        .iter()
        .map(list_entry)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ListGuardrailsResponse {
        guardrails: entries,
    }))
}

fn list_entry(guardrail: &ConfigGuardrail) -> Result<GuardrailInfoResponse, ReadError> {
    let params = guardrail.litellm_params.clone().unwrap_or_default();
    let masked = mask_sensitive_values(&params);
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        // LiteLLM validates even the empty dict, which yields the all-null default model
        litellm_params: Some(base_params(&masked)?),
        guardrail_info: guardrail.guardrail_info.clone(),
        created_at: None,
        updated_at: None,
        guardrail_definition_location: GuardrailDefinitionLocation::Config,
    })
}

pub(super) async fn list_guardrails_v2<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
) -> Result<Json<ListGuardrailsResponse>, ReadError>
where
    R: GuardrailRows,
{
    let context = authenticated(caller)?;
    let is_admin = has_admin_view(&context.principal);
    match v2_entries(&state, &context.principal, is_admin).await {
        Ok(guardrails) => Ok(Json(ListGuardrailsResponse { guardrails })),
        Err(error) => {
            tracing::error!("Error getting guardrails from db: {error}");
            Err(error)
        }
    }
}

async fn v2_entries<R>(
    state: &ReadState<R>,
    principal: &Principal,
    is_admin: bool,
) -> Result<Vec<GuardrailInfoResponse>, ReadError>
where
    R: GuardrailRows,
{
    let rows = if state.database {
        state
            .runtime
            .registry()
            .get_all_guardrails_from_db()
            .await?
    } else {
        Vec::new()
    };
    // LiteLLM never computes the caller's teams for an admin view
    let teams = if is_admin {
        Vec::new()
    } else {
        team_ids(state, principal).await?
    };
    let mut seen: HashSet<Option<String>> = HashSet::new();
    let mut allowed = Vec::with_capacity(rows.len());
    for guardrail in rows {
        // a row hidden from the caller stays in the seen set, so a config entry with its id is hidden too
        if is_admin || visible(guardrail.team_id.as_deref(), &teams) {
            allowed.push(guardrail);
        } else {
            seen.insert(guardrail.guardrail_id.clone());
        }
    }
    let mut entries = appended_rows(allowed, &mut seen)?;
    entries.extend(appended_memory(state, &teams, is_admin, &mut seen).await?);
    Ok(entries)
}

fn visible(team_id: Option<&str>, teams: &[String]) -> bool {
    team_id.is_none_or(|team| teams.iter().any(|candidate| candidate == team))
}

fn appended_rows(
    rows: Vec<Guardrail>,
    seen: &mut HashSet<Option<String>>,
) -> Result<Vec<GuardrailInfoResponse>, ReadError> {
    let mut entries = Vec::with_capacity(rows.len());
    for guardrail in rows {
        seen.insert(guardrail.guardrail_id.clone());
        entries.push(db_entry(&guardrail)?);
    }
    Ok(entries)
}

// a db row is a dict as stored, so its null params stay; its info does too
fn db_entry(guardrail: &Guardrail) -> Result<GuardrailInfoResponse, ReadError> {
    let dump = stored_dump(&guardrail.litellm_params)?;
    let masked = mask_sensitive_values(&dump);
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        litellm_params: validated(&masked)?,
        guardrail_info: guardrail.guardrail_info.clone(),
        created_at: guardrail.created_at,
        updated_at: guardrail.updated_at,
        guardrail_definition_location: GuardrailDefinitionLocation::Db,
    })
}

async fn appended_memory<R>(
    state: &ReadState<R>,
    teams: &[String],
    is_admin: bool,
    seen: &mut HashSet<Option<String>>,
) -> Result<Vec<GuardrailInfoResponse>, ReadError>
where
    R: GuardrailRows,
{
    let held: Vec<(Guardrail, Option<GuardrailDefinitionLocation>)> = {
        let handler = state.runtime.handler().read().await;
        handler
            .list_in_memory_guardrails()
            .into_iter()
            .map(|guardrail| {
                let source = guardrail
                    .guardrail_id
                    .as_deref()
                    .and_then(|guardrail_id| handler.get_source(guardrail_id));
                (guardrail, source)
            })
            .collect()
    };
    let mut entries = Vec::with_capacity(held.len());
    for (guardrail, source) in held {
        let guardrail_id = guardrail.guardrail_id.clone();
        if seen.contains(&guardrail_id) {
            continue;
        }
        // a row deleted on another pod leaves a db-sourced entry behind until reconciliation
        if guardrail_id.is_some() && source == Some(GuardrailDefinitionLocation::Db) {
            continue;
        }
        if !is_admin && !visible(guardrail.team_id.as_deref(), teams) {
            continue;
        }
        entries.push(config_entry(&guardrail)?);
        seen.insert(guardrail_id);
    }
    Ok(entries)
}

// an in-memory config entry is a typed LitellmParams instance in LiteLLM, so its nulls drop and its info becomes an object
fn config_entry(guardrail: &Guardrail) -> Result<GuardrailInfoResponse, ReadError> {
    let dump = config_dump(&guardrail.litellm_params)?;
    let masked = mask_sensitive_values(&dump);
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        litellm_params: validated(&masked)?,
        guardrail_info: Some(guardrail.guardrail_info.clone().unwrap_or_default()),
        created_at: None,
        updated_at: None,
        guardrail_definition_location: GuardrailDefinitionLocation::Config,
    })
}

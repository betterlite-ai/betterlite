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
use super::projection::{stored_dump, validated, without_none};
use super::state::ReadState;
use crate::auth::auth_identity::AuthContext;

pub(super) async fn list_guardrails_v2<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
) -> Result<Json<ListGuardrailsResponse>, ReadError>
where
    R: GuardrailRows,
{
    let context = authenticated(caller)?;
    let is_admin = has_admin_view(&context.principal);
    v2_response(&state, &context.principal, is_admin)
        .await
        .inspect_err(|error| tracing::error!("Error getting guardrails from db: {error}"))
}

async fn v2_response<R>(
    state: &ReadState<R>,
    principal: &Principal,
    is_admin: bool,
) -> Result<Json<ListGuardrailsResponse>, ReadError>
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
        let own_team = guardrail
            .team_id
            .as_deref()
            .is_none_or(|team| teams.iter().any(|candidate| candidate == team));
        if is_admin || own_team {
            allowed.push(guardrail);
        } else {
            seen.insert(guardrail.guardrail_id.clone());
        }
    }
    let mut entries = appended_rows(allowed, &mut seen)?;
    entries.extend(appended_memory(state, &teams, is_admin, &mut seen).await?);
    Ok(Json(ListGuardrailsResponse {
        guardrails: entries,
    }))
}

fn appended_rows(
    rows: Vec<Guardrail>,
    seen: &mut HashSet<Option<String>>,
) -> Result<Vec<GuardrailInfoResponse>, ReadError> {
    let mut entries = Vec::with_capacity(rows.len());
    for guardrail in rows {
        seen.insert(guardrail.guardrail_id.clone());
        entries.push(memory_entry(&guardrail, GuardrailDefinitionLocation::Db)?);
    }
    Ok(entries)
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
        let own_team = guardrail
            .team_id
            .as_deref()
            .is_none_or(|team| teams.iter().any(|candidate| candidate == team));
        if !is_admin && !own_team {
            continue;
        }
        entries.push(memory_entry(
            &guardrail,
            GuardrailDefinitionLocation::Config,
        )?);
        seen.insert(guardrail_id);
    }
    Ok(entries)
}

// a db row is a dict as stored, so its null params stay; a config entry is a typed LitellmParams, so its nulls drop
fn memory_entry(
    guardrail: &Guardrail,
    location: GuardrailDefinitionLocation,
) -> Result<GuardrailInfoResponse, ReadError> {
    let stored = stored_dump(&guardrail.litellm_params)?;
    let dump = match location {
        GuardrailDefinitionLocation::Db => stored,
        GuardrailDefinitionLocation::Config => without_none(&stored),
    };
    let masked = mask_sensitive_values(&dump);
    let guardrail_info = match location {
        GuardrailDefinitionLocation::Db => guardrail.guardrail_info.clone(),
        GuardrailDefinitionLocation::Config => {
            Some(guardrail.guardrail_info.clone().unwrap_or_default())
        }
    };
    let (created_at, updated_at) = match location {
        GuardrailDefinitionLocation::Db => (guardrail.created_at, guardrail.updated_at),
        GuardrailDefinitionLocation::Config => (None, None),
    };
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        litellm_params: validated(&masked)?,
        guardrail_info,
        created_at,
        updated_at,
        guardrail_definition_location: location,
    })
}

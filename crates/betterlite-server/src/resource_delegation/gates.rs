use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamFields;
use thiserror::Error;

use super::denial::Denial;
use super::quoted_list;
use super::requested::RequestedError;
use super::requested::requested_entries;

#[derive(Debug, Error)]
pub enum GateError {
    #[error(transparent)]
    Requested(#[from] RequestedError),
    #[error(transparent)]
    Denied(#[from] Denial),
}

pub fn validate_requested_toolsets(
    requested: &BTreeSet<String>,
    team: Option<&TeamFields>,
    is_proxy_admin: bool,
) -> Result<(), Denial> {
    if requested.is_empty() {
        return Ok(());
    }
    let Some(team_fields) = team else {
        if is_proxy_admin {
            return Ok(());
        }
        let disallowed_text = quoted_list(requested);
        return Err(Denial {
            detail: format!(
                "Key is not in a team. MCP toolsets cannot be assigned to personal keys by non-admin callers. Disallowed toolsets: {disallowed_text}."
            ),
        });
    };
    let team_toolsets: &[String] = team_fields
        .object_permission
        .as_ref()
        .and_then(|permission| permission.mcp_toolsets.as_deref())
        .unwrap_or_default();
    if team_toolsets.is_empty() {
        return Ok(());
    }
    let allowed: BTreeSet<String> = team_toolsets.iter().cloned().collect();
    let disallowed: BTreeSet<String> = requested.difference(&allowed).cloned().collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    let team_id = team_fields.base.team_id.as_str();
    let disallowed_text = quoted_list(&disallowed);
    let allowed_text = quoted_list(team_toolsets);
    Err(Denial {
        detail: format!(
            "Key requests MCP toolsets not allowed by team '{team_id}': {disallowed_text}. Team allows: {allowed_text}."
        ),
    })
}

pub fn validate_vector_stores(
    permission: Option<&JsonData>,
    team: Option<&TeamFields>,
    is_proxy_admin: bool,
) -> Result<(), GateError> {
    let requested: BTreeSet<String> = requested_entries("vector_stores", permission, true)?;
    if requested.is_empty() || team.is_some() || is_proxy_admin {
        return Ok(());
    }
    let disallowed_text = quoted_list(&requested);
    Err(GateError::Denied(Denial {
        detail: format!(
            "Key is not in a team. Vector stores cannot be assigned to personal keys by non-admin callers. Disallowed vector stores: {disallowed_text}."
        ),
    }))
}

pub fn validate_search_tools(
    permission: Option<&JsonData>,
    team: Option<&TeamFields>,
    is_proxy_admin: bool,
) -> Result<(), GateError> {
    let requested: Vec<String> = requested_entries("search_tools", permission, true)?;
    if requested.is_empty() {
        return Ok(());
    }
    let Some(team_fields) = team else {
        if is_proxy_admin {
            return Ok(());
        }
        let disallowed_text = quoted_list(&requested);
        return Err(GateError::Denied(Denial {
            detail: format!(
                "Key is not in a team. search_tools cannot be assigned to personal keys by non-admin callers. Disallowed search tools: {disallowed_text}."
            ),
        }));
    };
    let team_tools: &[String] = team_fields
        .object_permission
        .as_ref()
        .and_then(|stored| stored.search_tools.0.as_deref())
        .unwrap_or_default();
    if team_tools.is_empty() {
        return Ok(());
    }
    let allowed: BTreeSet<String> = team_tools.iter().cloned().collect();
    let disallowed: BTreeSet<String> = requested
        .iter()
        .filter(|tool| !allowed.contains(*tool))
        .cloned()
        .collect();
    if disallowed.is_empty() {
        return Ok(());
    }
    let team_id = team_fields.base.team_id.as_str();
    let disallowed_text = quoted_list(&disallowed);
    let allowed_text = quoted_list(team_tools);
    Err(GateError::Denied(Denial {
        detail: format!(
            "Key requests search tools not allowed by team '{team_id}': {disallowed_text}. Team allows: {allowed_text}."
        ),
    }))
}

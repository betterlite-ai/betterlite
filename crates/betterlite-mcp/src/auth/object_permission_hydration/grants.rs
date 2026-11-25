use std::collections::{BTreeMap, BTreeSet};

use super::error::{EntitlementError, UnloadableEntitlementError};
use super::principal::{key_object_permission_hydrated, team_object_permission};
use super::reads::{ObjectPermission, ObjectPermissionReads};
use super::record::{MCP_ALL_TOOLS_WILDCARD, McpAuth};

// a permission row that declares toolsets has a known restriction of unknown contents when they resolve to nothing
pub async fn toolset_tool_permissions<Reads>(
    reads: &Reads,
    permission: Option<&ObjectPermission>,
    requires_fresh_policy: bool,
) -> Result<BTreeMap<String, Vec<String>>, EntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let Some(toolset_ids) = permission
        .and_then(|row| row.mcp_toolsets.as_deref())
        .filter(|ids| !ids.is_empty())
    else {
        return Ok(BTreeMap::new());
    };
    let grants = reads
        .resolve_toolset_grants(toolset_ids, requires_fresh_policy)
        .await
        .map_err(|source| EntitlementError::Read { source })?;
    if grants.is_empty() {
        return Err(EntitlementError::Unloadable(
            UnloadableEntitlementError::toolsets(toolset_ids),
        ));
    }
    Ok(grants)
}

pub async fn toolset_tools_for_server<Reads>(
    reads: &Reads,
    permission: Option<&ObjectPermission>,
    server_id: &str,
    requires_fresh_policy: bool,
) -> Result<Option<Vec<String>>, EntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    Ok(
        toolset_tool_permissions(reads, permission, requires_fresh_policy)
            .await?
            .remove(server_id),
    )
}

#[must_use]
pub fn union_tool_grants(
    direct: Option<&[String]>,
    via_toolsets: Option<&[String]>,
) -> Option<Vec<String>> {
    if direct.is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool.as_str() == MCP_ALL_TOOLS_WILDCARD)
    }) {
        return None;
    }
    if direct.is_none() && via_toolsets.is_none() {
        return None;
    }
    let mut granted = BTreeSet::new();
    granted.extend(direct.into_iter().flatten().map(String::as_str));
    granted.extend(via_toolsets.into_iter().flatten().map(String::as_str));
    Some(granted.into_iter().map(str::to_owned).collect())
}

pub async fn key_or_team_declares_toolsets<Reads>(reads: &Reads, auth: Option<&McpAuth>) -> bool
where
    Reads: ObjectPermissionReads,
{
    let Some(record) = auth else {
        return false;
    };
    match any_toolsets_declared(reads, record).await {
        Ok(declares) => declares,
        Err(source) => {
            tracing::warn!(
                "Failed to check declared MCP toolsets, the org ceiling is unchanged: {source}"
            );
            false
        }
    }
}

async fn any_toolsets_declared<Reads>(reads: &Reads, auth: &McpAuth) -> Result<bool, Reads::Error>
where
    Reads: ObjectPermissionReads,
{
    let key_permission = key_object_permission_hydrated(reads, auth).await?;
    if declares_toolsets(key_permission.as_ref()) {
        return Ok(true);
    }
    if auth.team_id.is_none() {
        return Ok(false);
    }
    let team_permission = team_object_permission(reads, auth).await?;
    Ok(declares_toolsets(team_permission.as_ref()))
}

fn declares_toolsets(permission: Option<&ObjectPermission>) -> bool {
    let Some(row) = permission else {
        return false;
    };
    row.mcp_toolsets
        .as_ref()
        .is_some_and(|toolset_ids| !toolset_ids.is_empty())
}

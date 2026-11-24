use std::error::Error as StdError;

use betterlite_auth::auth_cache::keys::{CacheKey, USER_NO_MCP_PERMISSION};
use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;

use super::error::{EntitlementError, UnloadableEntitlementError, UserPermissionError};
use super::hydration::load_named_object_permission;
use super::reads::{CachedLink, ObjectPermission, ObjectPermissionReads, OrgPermissionLink};
use super::record::{AGENT_NO_MCP_PERMISSION_SENTINEL, McpAuth};

const UI_TEAM_ID: &str = UI_SESSION_TOKEN_TEAM_ID;

pub async fn team_object_permission<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, Reads::Error>
where
    Reads: ObjectPermissionReads,
{
    let Some(team_id) = auth.team_id.as_deref() else {
        return Ok(None);
    };
    if !reads.has_database() || team_id == UI_TEAM_ID {
        return Ok(None);
    }
    reads
        .team_permission(team_id, auth.requires_fresh_policy)
        .await
}

pub async fn key_object_permission_hydrated<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, Reads::Error>
where
    Reads: ObjectPermissionReads,
{
    if let Some(loaded) = &auth.object_permission {
        return Ok(Some(loaded.clone()));
    }
    let Some(object_permission_id) = auth.object_permission_id.as_deref() else {
        return Ok(None);
    };
    if !reads.has_database() {
        return Ok(None);
    }
    reads
        .permission(object_permission_id, auth.requires_fresh_policy)
        .await
}

pub async fn org_object_permission<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, EntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let Some(org_id) = auth.org_id.as_deref() else {
        return Ok(None);
    };
    if !reads.has_database() {
        return Ok(None);
    }
    let link = reads
        .org_object_permission_link(org_id)
        .await
        .map_err(|source| EntitlementError::Read { source })?;
    let OrgPermissionLink::Named(object_permission_id) = link else {
        return Ok(None);
    };
    let principal = format!("org {org_id:?}");
    load_named_object_permission(
        reads,
        &principal,
        &object_permission_id,
        auth.requires_fresh_policy,
    )
    .await
    .map(Some)
    .map_err(EntitlementError::from)
}

pub async fn end_user_object_permission<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, UnloadableEntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let Some(end_user_id) = auth.end_user_id.as_deref() else {
        return Ok(None);
    };
    let entitlement = match reads.end_user_entitlement(end_user_id).await {
        Ok(found) => found,
        Err(source) => {
            tracing::warn!("Failed to resolve end_user for MCP permissions: {source}");
            return Ok(None);
        }
    };
    let Some(found) = entitlement else {
        return Ok(None);
    };
    if let Some(permission) = found.object_permission {
        return Ok(Some(permission));
    }
    let Some(object_permission_id) = found.object_permission_id else {
        return Ok(None);
    };
    let principal = format!("end user {end_user_id:?}");
    load_named_object_permission(
        reads,
        &principal,
        &object_permission_id,
        auth.requires_fresh_policy,
    )
    .await
    .map(Some)
}

pub async fn user_object_permission_id<Reads>(
    reads: &Reads,
    user_id: &str,
    check_db_only: bool,
) -> Result<Option<String>, UserPermissionError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let cache_key = CacheKey::UserObjectPermissionId(user_id).render();
    if !check_db_only {
        let cached = match reads.cached_link(&cache_key).await {
            Ok(found) => found,
            Err(source) => return link_failure(source, check_db_only, user_id),
        };
        match cached {
            Some(CachedLink::NoPermission) => return Ok(None),
            Some(CachedLink::Id(id)) => return Ok(Some(id)),
            Some(CachedLink::NotAPermissionLink) | None => {}
        }
    }
    let linked = match reads
        .user_object_permission_link(user_id, check_db_only)
        .await
    {
        Ok(found) => found,
        Err(source) => return link_failure(source, check_db_only, user_id),
    };
    let object_permission_id = linked;
    let cached_value = object_permission_id
        .clone()
        .unwrap_or_else(|| USER_NO_MCP_PERMISSION.to_owned());
    let ttl = reads.management_ttl();
    if let Err(source) = reads.store_link(&cache_key, &cached_value, ttl).await {
        return link_failure(source, check_db_only, user_id);
    }
    Ok(object_permission_id)
}

pub async fn user_object_permission<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, UserPermissionError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let Some(user_id) = auth.user_id.as_deref() else {
        return Ok(None);
    };
    if !reads.has_database() {
        return Ok(None);
    }
    let Some(named_id) =
        user_object_permission_id(reads, user_id, auth.requires_fresh_policy).await?
    else {
        return Ok(None);
    };
    let permission = reads
        .permission(&named_id, auth.requires_fresh_policy)
        .await
        .map_err(|source| UserPermissionError::Read { source })?;
    match permission {
        Some(found) => Ok(Some(found)),
        None => Err(UserPermissionError::Missing {
            user_id: user_id.to_owned(),
            object_permission_id: named_id,
        }),
    }
}

pub async fn agent_object_permission_id<Reads>(reads: &Reads, agent_id: &str) -> Option<String>
where
    Reads: ObjectPermissionReads,
{
    match resolve_agent_link(reads, agent_id).await {
        Ok(link) => link,
        Err(source) => {
            tracing::warn!(
                "Failed to resolve object_permission_id for agent {agent_id:?}: {source}"
            );
            None
        }
    }
}

pub async fn agent_object_permission<Reads>(
    reads: &Reads,
    auth: &McpAuth,
) -> Result<Option<ObjectPermission>, UnloadableEntitlementError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let Some(agent_id) = auth.agent_id.as_deref() else {
        return Ok(None);
    };
    if !reads.has_database() {
        return Ok(None);
    }
    let Some(object_permission_id) = agent_object_permission_id(reads, agent_id).await else {
        return Ok(None);
    };
    let principal = format!("agent {agent_id:?}");
    load_named_object_permission(
        reads,
        &principal,
        &object_permission_id,
        auth.requires_fresh_policy,
    )
    .await
    .map(Some)
}

async fn resolve_agent_link<Reads>(
    reads: &Reads,
    agent_id: &str,
) -> Result<Option<String>, Reads::Error>
where
    Reads: ObjectPermissionReads,
{
    let cache_key = format!("agent_object_permission_id:{agent_id}");
    match reads.cached_link(&cache_key).await? {
        Some(CachedLink::NoPermission) => return Ok(None),
        Some(CachedLink::Id(id)) => return Ok(Some(id)),
        Some(CachedLink::NotAPermissionLink) | None => {}
    }
    let linked = reads.agent_object_permission_link(agent_id).await?;
    let object_permission_id = linked;
    let cached_value = object_permission_id
        .clone()
        .unwrap_or_else(|| AGENT_NO_MCP_PERMISSION_SENTINEL.to_owned());
    let ttl = reads.management_ttl();
    reads.store_link(&cache_key, &cached_value, ttl).await?;
    Ok(object_permission_id)
}

fn link_failure<Source>(
    source: Source,
    check_db_only: bool,
    user_id: &str,
) -> Result<Option<String>, UserPermissionError<Source>>
where
    Source: StdError + 'static,
{
    if check_db_only {
        return Err(UserPermissionError::Unavailable { source });
    }
    tracing::warn!("MCP user entitlement: link for {user_id:?} unresolved, no ceiling: {source}");
    Ok(None)
}

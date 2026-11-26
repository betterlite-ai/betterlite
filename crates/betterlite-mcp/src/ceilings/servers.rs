use indexmap::IndexSet;

use super::error::CeilingReadError;
use super::reads::CeilingReads;
use crate::auth::object_permission_hydration::reads::ObjectPermission;

// whether a permission's declared toolsets join its server union: they do at every level but the end user's
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToolsetTerm {
    Counted,
    Skipped,
}

// every server a permission grants at one level: its expanded list, its access groups, its tool servers and its toolset servers
pub(super) async fn permission_servers<Reads>(
    reads: &Reads,
    permission: &ObjectPermission,
    requires_fresh_policy: bool,
    toolsets: ToolsetTerm,
) -> Result<Vec<String>, CeilingReadError<Reads::Error>>
where
    Reads: CeilingReads,
{
    let mut server_ids: IndexSet<String> = IndexSet::new();
    let servers = permission.mcp_servers.0.as_deref().unwrap_or_default();
    server_ids.extend(reads.expand_permission_list(servers));
    let groups = permission
        .mcp_access_groups
        .0
        .as_deref()
        .unwrap_or_default();
    server_ids.extend(
        reads
            .servers_from_access_groups(groups, requires_fresh_policy)
            .await?,
    );
    server_ids.extend(
        reads
            .expand_tool_permissions(permission.mcp_tool_permissions.as_ref())
            .into_keys(),
    );
    if matches!(toolsets, ToolsetTerm::Counted) {
        let grants = reads
            .toolset_tool_permissions(Some(permission), requires_fresh_policy)
            .await?;
        server_ids.extend(grants.into_keys());
    }
    Ok(server_ids.into_iter().collect())
}

// a ceiling caps by membership and keeps the current list's order
pub(super) fn capped(allowed: Vec<String>, ceiling: &[String]) -> Vec<String> {
    allowed
        .into_iter()
        .filter(|server| ceiling.contains(server))
        .collect()
}

// LiteLLM answers these lists from a Python set, so the first sighting of an id wins and the order is ours to keep
pub(super) fn distinct(items: Vec<String>) -> Vec<String> {
    let mut unique: IndexSet<String> = IndexSet::new();
    unique.extend(items);
    unique.into_iter().collect()
}

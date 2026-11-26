use std::collections::BTreeMap;

use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ModelAccessObject;
use secrecy::ExposeSecret as _;

use crate::auth_model_lists::lists::dedup_first;
use crate::auth_model_patterns::access_groups::{
    AccessGroupError, AccessGroupLookup, AccessGroupResources,
};
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::ModelAccessDecision;
use crate::auth_model_patterns::grant::key_model_aliases;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::access::get_access_object;
use crate::auth_team_lookup::lookup::TeamLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::EntityModelAccessError;
use super::grant::entity_grant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    ModelNames,
    McpServerIds,
    AgentIds,
}

impl ResourceKind {
    // GETATTR(ag, resource_field) in get_authorized_resources_from_key_access_groups
    fn column(self, group: &AccessGroupTable) -> &[String] {
        match self {
            Self::ModelNames => &group.access_model_names,
            Self::McpServerIds => &group.access_mcp_server_ids,
            Self::AgentIds => &group.access_agent_ids,
        }
    }
}

// the access groups one request expands, read once and handed to B747's collector unchanged
#[derive(Debug, Default)]
pub struct GroupResources {
    groups: BTreeMap<String, AccessGroupResources>,
}

impl GroupResources {
    // _get_resources_from_access_groups: an unreadable group contributes nothing to the expansion
    pub async fn load(lookup: Option<&TeamLookup>, ids: &[String]) -> Self {
        let mut groups = BTreeMap::new();
        let Some(lookup) = lookup else {
            return Self { groups };
        };
        for id in ids {
            let Ok(table) = get_access_object(lookup, id, false).await else {
                continue;
            };
            groups.insert(
                id.clone(),
                AccessGroupResources {
                    model_names: table.access_model_names,
                    mcp_server_ids: table.access_mcp_server_ids,
                    agent_ids: table.access_agent_ids,
                },
            );
        }
        Self { groups }
    }
}

impl AccessGroupLookup for GroupResources {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        let found = self.groups.get(access_group_id).cloned();
        found.ok_or_else(|| AccessGroupError::Unavailable {
            access_group_id: access_group_id.to_owned(),
        })
    }
}

// get_authorized_resources_from_key_access_groups: a group counts only when the key's team or the key itself owns it
pub async fn authorized_resources(
    token: Option<&UserAPIKeyAuth>,
    team: Option<&LiteLLMTeamTable>,
    lookup: Option<&TeamLookup>,
    kind: ResourceKind,
) -> Vec<String> {
    let Some(token) = token else {
        return Vec::new();
    };
    let ids = token
        .view
        .token
        .access_group_ids
        .clone()
        .unwrap_or_default();
    if ids.is_empty() {
        return Vec::new();
    }
    let Some(lookup) = lookup else {
        return Vec::new();
    };
    // the key's own team wins; the supplied team object only answers when the key names none
    let key_team_id = token
        .view
        .token
        .team_id
        .as_deref()
        .or_else(|| team.map(|row| row.fields.base.team_id.as_str()));
    let key_hash = token
        .view
        .token
        .token
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    let mut resources: Vec<String> = Vec::new();
    for id in &ids {
        let Ok(group) = get_access_object(lookup, id, false).await else {
            continue;
        };
        let team_authorized = key_team_id
            .is_some_and(|team_id| group.assigned_team_ids.iter().any(|held| held == team_id));
        let key_authorized = key_hash
            .as_ref()
            .is_some_and(|hash| group.assigned_key_ids.iter().any(|held| held == hash));
        if team_authorized || key_authorized {
            resources.extend(kind.column(&group).iter().cloned());
        }
    }
    dedup_first(resources)
}

// _key_access_group_grants_model: only a ProxyException denial answers false; anything else escapes
pub async fn key_access_group_grants_model(
    model: &RequestedModels,
    token: Option<&UserAPIKeyAuth>,
    team: Option<&LiteLLMTeamTable>,
    lookup: Option<&TeamLookup>,
    context: &ModelAccessContext<'_>,
) -> Result<bool, EntityModelAccessError> {
    let resources = authorized_resources(token, team, lookup, ResourceKind::ModelNames).await;
    if resources.is_empty() {
        return Ok(false);
    }
    let aliases = token
        .and_then(|row| row.view.team_model_aliases.as_ref())
        .map(|payload| key_model_aliases(&payload.0));
    let targets = token.map(|row| key_model_aliases(&row.view.token.aliases.0));
    let grant = entity_grant(
        resources,
        aliases,
        targets,
        token.and_then(|row| row.view.token.team_id.clone()),
    );
    match can_object_call_model(model, &grant, ModelAccessObject::Key, context, 0) {
        ModelAccessDecision::Allowed => Ok(true),
        ModelAccessDecision::Denied(_) => Ok(false),
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            Err(EntityModelAccessError::RecurseDepth { model })
        }
    }
}

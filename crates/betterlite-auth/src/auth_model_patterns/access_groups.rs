use std::fmt::Debug;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceField {
    ModelNames,
    McpServerIds,
    AgentIds,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessGroupResources {
    pub model_names: Vec<String>,
    pub mcp_server_ids: Vec<String>,
    pub agent_ids: Vec<String>,
}

#[derive(Debug, Error)]
pub enum AccessGroupError {
    #[error("the access group {access_group_id} could not be loaded")]
    Unavailable { access_group_id: String },
}

pub trait AccessGroupLookup: Debug + Send + Sync {
    fn access_group(&self, access_group_id: &str)
    -> Result<AccessGroupResources, AccessGroupError>;
}

#[must_use]
pub fn collect_resources(
    access_group_ids: &[String],
    resource_field: ResourceField,
    lookup: Option<&dyn AccessGroupLookup>,
    check_db_only: bool,
) -> Result<Vec<String>, AccessGroupError> {
    if access_group_ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(lookup) = lookup else {
        return Ok(Vec::new());
    };
    let mut resources: Vec<String> = Vec::new();
    for access_group_id in access_group_ids {
        let group = match lookup.access_group(access_group_id) {
            Ok(group) => group,
            Err(error) => {
                if check_db_only {
                    return Err(error);
                }
                continue;
            }
        };
        let group_resources: &[String] = match resource_field {
            ResourceField::ModelNames => &group.model_names,
            ResourceField::McpServerIds => &group.mcp_server_ids,
            ResourceField::AgentIds => &group.agent_ids,
        };
        for resource in group_resources {
            if !resources.contains(resource) {
                resources.push(resource.clone());
            }
        }
    }
    Ok(resources)
}

#[must_use]
pub fn models_from_access_groups(
    access_group_ids: &[String],
    lookup: Option<&dyn AccessGroupLookup>,
) -> Vec<String> {
    collect_resources(access_group_ids, ResourceField::ModelNames, lookup, false)
        .unwrap_or_default()
}

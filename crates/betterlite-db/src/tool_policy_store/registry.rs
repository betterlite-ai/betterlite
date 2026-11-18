use std::collections::{BTreeMap, BTreeSet};

use crate::tool_records::records::{LiteLLMToolTableRow, ToolInputPolicy, ToolOutputPolicy};

use super::records::BlockedToolRow;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPolicyMaps {
    pub input_policies: BTreeMap<String, ToolInputPolicy>,
    pub output_policies: BTreeMap<String, ToolOutputPolicy>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPolicyRegistry {
    pub input_policies: BTreeMap<String, ToolInputPolicy>,
    pub output_policies: BTreeMap<String, ToolOutputPolicy>,
    pub blocked_tools_by_permission: BTreeMap<String, Vec<String>>,
    pub initialized: bool,
}

impl ToolPolicyRegistry {
    #[must_use]
    pub fn effective_policies(
        &self,
        tool_names: &[String],
        key_permission_id: Option<&str>,
        team_permission_id: Option<&str>,
    ) -> BTreeMap<String, ToolInputPolicy> {
        let mut blocked = BTreeSet::new();
        for permission_id in [key_permission_id, team_permission_id]
            .into_iter()
            .flatten()
        {
            if let Some(tools) = self.blocked_tools_by_permission.get(permission_id.trim()) {
                blocked.extend(tools.iter().map(String::as_str));
            }
        }
        let mut policies = BTreeMap::new();
        for name in tool_names {
            let policy = if blocked.contains(name.as_str()) {
                ToolInputPolicy::Blocked
            } else {
                self.input_policies
                    .get(name.as_str())
                    .copied()
                    .unwrap_or_default()
            };
            policies.insert(name.clone(), policy);
        }
        policies
    }

    #[must_use]
    pub fn input_policy(&self, tool_name: &str) -> ToolInputPolicy {
        match self.input_policies.get(tool_name) {
            Some(policy) => *policy,
            None => ToolInputPolicy::Untrusted,
        }
    }

    #[must_use]
    pub fn output_policy(&self, tool_name: &str) -> ToolOutputPolicy {
        match self.output_policies.get(tool_name) {
            Some(policy) => *policy,
            None => ToolOutputPolicy::Untrusted,
        }
    }
}

#[must_use]
pub fn tool_policy_maps(tools: &[LiteLLMToolTableRow]) -> ToolPolicyMaps {
    let mut maps = ToolPolicyMaps::default();
    for tool in tools {
        maps.input_policies
            .insert(tool.tool_name.clone(), tool.input_policy);
        maps.output_policies
            .insert(tool.tool_name.clone(), tool.output_policy);
    }
    maps
}

#[must_use]
pub fn blocked_tools_map(rows: &[BlockedToolRow]) -> BTreeMap<String, Vec<String>> {
    let mut blocked = BTreeMap::new();
    for row in rows {
        if row.object_permission_id.is_empty() {
            continue;
        }
        blocked.insert(
            row.object_permission_id.clone(),
            row.blocked_tools.clone().unwrap_or_default(),
        );
    }
    blocked
}

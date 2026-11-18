use std::collections::BTreeMap;

use betterlite_db::tool_policy_store::records::BlockedToolRow;
use betterlite_db::tool_policy_store::registry::{
    ToolPolicyRegistry, blocked_tools_map, tool_policy_maps,
};
use betterlite_db::tool_records::records::{
    LiteLLMToolTableRow, ToolInputPolicy, ToolOutputPolicy,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Query {
    tool_names: Vec<String>,
    #[serde(default)]
    key_permission_id: Option<String>,
    #[serde(default)]
    team_permission_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    tools: Vec<LiteLLMToolTableRow>,
    #[serde(default)]
    permissions: Vec<BlockedToolRow>,
    queries: Vec<Query>,
    #[serde(default)]
    output_tools: Vec<String>,
}

#[derive(serde::Serialize)]
struct QueryAnswer {
    tool_names: Vec<String>,
    key_permission_id: Option<String>,
    team_permission_id: Option<String>,
    effective: BTreeMap<String, ToolInputPolicy>,
}

#[derive(serde::Serialize)]
struct Output {
    initialized: bool,
    queries: Vec<QueryAnswer>,
    output_policies: BTreeMap<String, ToolOutputPolicy>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let maps = tool_policy_maps(&input.tools);
        let registry = ToolPolicyRegistry {
            input_policies: maps.input_policies,
            output_policies: maps.output_policies,
            blocked_tools_by_permission: blocked_tools_map(&input.permissions),
            initialized: true,
        };
        let mut queries = Vec::with_capacity(input.queries.len());
        for query in &input.queries {
            queries.push(QueryAnswer {
                tool_names: query.tool_names.clone(),
                key_permission_id: query.key_permission_id.clone(),
                team_permission_id: query.team_permission_id.clone(),
                effective: registry.effective_policies(
                    &query.tool_names,
                    query.key_permission_id.as_deref(),
                    query.team_permission_id.as_deref(),
                ),
            });
        }
        let mut output_policies = BTreeMap::new();
        for name in &input.output_tools {
            output_policies.insert(name.clone(), registry.output_policy(name));
        }
        serde_json::to_value(Output {
            initialized: true,
            queries,
            output_policies,
        })
        .or_value_error()
    }
}

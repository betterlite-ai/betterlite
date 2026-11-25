use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::resource_delegation::ObjectPermissionGrants;
use betterlite_server::resource_delegation::TeamGrants;
use betterlite_server::resource_delegation::denial::Denial;
use betterlite_server::resource_delegation::gates::GateError;
use betterlite_server::resource_delegation::gates::validate_requested_toolsets;
use betterlite_server::resource_delegation::gates::validate_search_tools;
use betterlite_server::resource_delegation::gates::validate_vector_stores;
use betterlite_server::resource_delegation::requested::requested_entries;
use betterlite_server::resource_delegation::requested::requested_server_ids;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeamInput {
    team_id: String,
    #[serde(default)]
    mcp_toolsets: Option<Vec<String>>,
    #[serde(default)]
    search_tools: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ExtractMcpServers {
        #[serde(default)]
        permission: Option<JsonData>,
    },
    ExtractMcpAccessGroups {
        #[serde(default)]
        permission: Option<JsonData>,
    },
    ExtractMcpToolsets {
        #[serde(default)]
        permission: Option<JsonData>,
    },
    ExtractVectorStores {
        #[serde(default)]
        permission: Option<JsonData>,
    },
    ExtractSearchTools {
        #[serde(default)]
        permission: Option<JsonData>,
    },
    ValidateToolsets(GateInput),
    ValidateVectorStores(GateInput),
    ValidateSearchTools(GateInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateInput {
    #[serde(default)]
    permission: Option<JsonData>,
    #[serde(default)]
    team: Option<TeamInput>,
    #[serde(default)]
    is_proxy_admin: bool,
}

impl GateInput {
    fn evaluate(&self, op: &GateOp) -> HelperResult {
        run_gate(
            self.permission.as_ref(),
            self.team.as_ref(),
            self.is_proxy_admin,
            op,
        )
    }
}

fn team_of(input: &TeamInput) -> TeamGrants {
    TeamGrants {
        team_id: input.team_id.clone(),
        access_group_ids: Vec::new(),
        object_permission: Some(ObjectPermissionGrants {
            mcp_servers: None,
            mcp_access_groups: None,
            mcp_tool_permissions: None,
            mcp_toolsets: input.mcp_toolsets.clone(),
            search_tools: input.search_tools.clone(),
        }),
    }
}

fn denial_error(denial: &Denial) -> HelperError {
    HelperError {
        class: "HTTPException".to_owned(),
        status: Some(Denial::STATUS_CODE),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: denial.detail.clone(),
    }
}

fn gate_result(result: Result<(), GateError>) -> HelperResult {
    match result {
        Ok(()) => Ok(Value::Null),
        Err(GateError::Denied(denial)) => Err(Box::new(denial_error(&denial))),
        Err(GateError::Requested(error)) => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })),
    }
}

fn extract_list<Collection>(
    field: &'static str,
    permission: Option<&JsonData>,
    drop_empty: bool,
) -> HelperResult
where
    Collection: FromIterator<String> + Serialize,
{
    Ok(serde_json::to_value(
        requested_entries::<Collection>(field, permission, drop_empty).or_value_error()?,
    )?)
}

enum GateOp {
    Toolsets,
    VectorStores,
    SearchTools,
}

fn run_gate(
    permission: Option<&JsonData>,
    team: Option<&TeamInput>,
    is_proxy_admin: bool,
    op: &GateOp,
) -> HelperResult {
    let grants = team.map(team_of);
    match op {
        GateOp::Toolsets => {
            let requested: BTreeSet<String> =
                requested_entries("mcp_toolsets", permission, false).or_value_error()?;
            gate_result(
                validate_requested_toolsets(&requested, grants.as_ref(), is_proxy_admin)
                    .map_err(GateError::from),
            )
        }
        GateOp::VectorStores => gate_result(validate_vector_stores(
            permission,
            grants.as_ref(),
            is_proxy_admin,
        )),
        GateOp::SearchTools => gate_result(validate_search_tools(
            permission,
            grants.as_ref(),
            is_proxy_admin,
        )),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExtractMcpServers { permission } => Ok(serde_json::to_value(
                requested_server_ids(permission.as_ref()).or_value_error()?,
            )?),
            Input::ExtractMcpAccessGroups { permission } => {
                extract_list::<BTreeSet<String>>("mcp_access_groups", permission.as_ref(), false)
            }
            Input::ExtractMcpToolsets { permission } => {
                extract_list::<BTreeSet<String>>("mcp_toolsets", permission.as_ref(), false)
            }
            Input::ExtractVectorStores { permission } => {
                extract_list::<BTreeSet<String>>("vector_stores", permission.as_ref(), true)
            }
            Input::ExtractSearchTools { permission } => {
                extract_list::<Vec<String>>("search_tools", permission.as_ref(), true)
            }
            Input::ValidateToolsets(gate) => gate.evaluate(&GateOp::Toolsets),
            Input::ValidateVectorStores(gate) => gate.evaluate(&GateOp::VectorStores),
            Input::ValidateSearchTools(gate) => gate.evaluate(&GateOp::SearchTools),
        }
    }
}

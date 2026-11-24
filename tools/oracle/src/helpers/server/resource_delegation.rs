use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::resource_delegation::ObjectPermissionGrants;
use betterlite_server::resource_delegation::TeamGrants;
use betterlite_server::resource_delegation::denial::Denial;
use betterlite_server::resource_delegation::gates::GateError;
use betterlite_server::resource_delegation::gates::validate_search_tools;
use betterlite_server::resource_delegation::gates::validate_toolsets;
use betterlite_server::resource_delegation::gates::validate_vector_stores;
use betterlite_server::resource_delegation::requested::RequestedError;
use betterlite_server::resource_delegation::requested::requested_access_groups;
use betterlite_server::resource_delegation::requested::requested_search_tools;
use betterlite_server::resource_delegation::requested::requested_server_ids;
use betterlite_server::resource_delegation::requested::requested_toolsets;
use betterlite_server::resource_delegation::requested::requested_vector_stores;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamInput {
    team_id: String,
    #[serde(default)]
    mcp_toolsets: Option<Vec<String>>,
    #[serde(default)]
    search_tools: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
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
    ValidateToolsets {
        #[serde(default)]
        permission: Option<JsonData>,
        #[serde(default)]
        team: Option<TeamInput>,
        #[serde(default)]
        is_proxy_admin: bool,
    },
    ValidateVectorStores {
        #[serde(default)]
        permission: Option<JsonData>,
        #[serde(default)]
        team: Option<TeamInput>,
        #[serde(default)]
        is_proxy_admin: bool,
    },
    ValidateSearchTools {
        #[serde(default)]
        permission: Option<JsonData>,
        #[serde(default)]
        team: Option<TeamInput>,
        #[serde(default)]
        is_proxy_admin: bool,
    },
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

fn extracted_value<T: Serialize>(result: Result<T, RequestedError>) -> HelperResult {
    let value = result.or_value_error()?;
    Ok(serde_json::to_value(value)?)
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

fn run_gate(
    permission: &Option<JsonData>,
    team: &Option<TeamInput>,
    is_proxy_admin: bool,
    validate: fn(Option<&JsonData>, Option<&TeamGrants>, bool) -> Result<(), GateError>,
) -> HelperResult {
    let grants = team.as_ref().map(team_of);
    gate_result(validate(
        permission.as_ref(),
        grants.as_ref(),
        is_proxy_admin,
    ))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExtractMcpServers { permission } => {
                extracted_value(requested_server_ids(permission.as_ref()))
            }
            Input::ExtractMcpAccessGroups { permission } => {
                extracted_value(requested_access_groups(permission.as_ref()))
            }
            Input::ExtractMcpToolsets { permission } => {
                extracted_value(requested_toolsets(permission.as_ref()))
            }
            Input::ExtractVectorStores { permission } => {
                extracted_value(requested_vector_stores(permission.as_ref()))
            }
            Input::ExtractSearchTools { permission } => {
                extracted_value(requested_search_tools(permission.as_ref()))
            }
            Input::ValidateToolsets {
                permission,
                team,
                is_proxy_admin,
            } => run_gate(permission, team, *is_proxy_admin, validate_toolsets),
            Input::ValidateVectorStores {
                permission,
                team,
                is_proxy_admin,
            } => run_gate(permission, team, *is_proxy_admin, validate_vector_stores),
            Input::ValidateSearchTools {
                permission,
                team,
                is_proxy_admin,
            } => run_gate(permission, team, *is_proxy_admin, validate_search_tools),
        }
    }
}

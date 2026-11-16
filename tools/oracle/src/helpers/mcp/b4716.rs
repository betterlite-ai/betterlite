use betterlite_config::sections::mcp_env_values::{PayloadRules, StdioPolicy};
use betterlite_db::mcp::table_access::McpIdentifierConflict;
use betterlite_db::mcp::table_access::types::IdentifierField;
use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_mcp::connector_import::convert::{
    ConnectorOutcome, convert_connector_entries, sanitize_connector_name,
};
use betterlite_mcp::connector_import::request::ConnectorImportRequest;
use betterlite_mcp::server_payload::names::validate_and_normalize_payload;
use betterlite_mcp::server_payload::request::{NewMCPServerRequest, UpdateMCPServerRequest};
// ast-grep-ignore: no-long-names
use betterlite_mcp::server_payload::request::is_per_server_oauth_discovery_eligible as discovery_eligible;
use betterlite_mcp::server_payload::rules::{
    identifier_conflict_message, stamp_omitted_oauth2_flow,
};
use betterlite_mcp::server_payload::{PayloadError, RequestError};
use betterlite_mcp::server_record::OAuth2Flow;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub(crate) struct Op;
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ValidatePayload(PayloadInput),
    NewRequest(PayloadInput),
    UpdateRequest(PayloadInput),
    StampFlow(PayloadInput),
    Eligibility {
        auth_type: Option<MCPAuth>,
        oauth2_flow: Option<OAuth2Flow>,
        delegate_auth_to_upstream: bool,
    },
    ConflictMessage {
        field: String,
        value: String,
        server_id: String,
    },
    SanitizeName {
        name: String,
    },
    ConvertConnectors {
        connectors: Json,
        stdio: StdioPolicy,
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadInput {
    payload: Json,
    stdio: StdioPolicy,
}
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ValidatePayload(call) => validate_payload(call),
            Input::NewRequest(call) => {
                serialized(NewMCPServerRequest::from_value(&call.payload, &call.stdio))
            }
            Input::UpdateRequest(call) => serialized(UpdateMCPServerRequest::from_value(
                &call.payload,
                &call.stdio,
            )),
            Input::StampFlow(call) => serialized(
                NewMCPServerRequest::from_value(&call.payload, &call.stdio).map(|mut request| {
                    stamp_omitted_oauth2_flow(&mut request);
                    request
                }),
            ),
            Input::Eligibility {
                auth_type,
                oauth2_flow,
                delegate_auth_to_upstream,
            } => Ok(json!({
                "eligible": discovery_eligible(
                    *auth_type,
                    *oauth2_flow,
                    *delegate_auth_to_upstream,
                ),
            })),
            Input::ConflictMessage {
                field,
                value,
                server_id,
            } => Ok(conflict_message(field, value, server_id)),
            Input::SanitizeName { name } => Ok(json!({ "name": sanitize_connector_name(name) })),
            Input::ConvertConnectors { connectors, stdio } => convert_connectors(connectors, stdio),
        }
    }
}
fn serialized<T: Serialize>(built: Result<T, RequestError>) -> HelperResult {
    match built {
        Ok(request) => Ok(serde_json::to_value(&request)?),
        Err(error) => Ok(request_error(error)),
    }
}
fn validate_payload(call: &PayloadInput) -> HelperResult {
    let mut request = match NewMCPServerRequest::from_value(&call.payload, &call.stdio) {
        Ok(request) => request,
        Err(error) => return Ok(request_error(error)),
    };
    let rules = PayloadRules {
        prefix_separator: "-".to_owned(),
        stdio: call.stdio.clone(),
    };
    match validate_and_normalize_payload(&mut request, &rules) {
        Ok(()) => Ok(serde_json::to_value(&request)?),
        Err(error) => Ok(validation_error(&error)),
    }
}
fn conflict_message(field: &str, value: &str, server_id: &str) -> Value {
    let conflict = McpIdentifierConflict {
        field: if field == "alias" {
            IdentifierField::Alias
        } else {
            IdentifierField::ServerName
        },
        value: value.to_owned(),
        server_id: server_id.to_owned(),
    };
    json!({ "message": identifier_conflict_message(&conflict) })
}
fn convert_connectors(connectors: &Json, stdio: &StdioPolicy) -> HelperResult {
    let request = ConnectorImportRequest::from_slice(&serde_json::to_vec(connectors)?)?;
    let outcomes = convert_connector_entries(&request, stdio);
    let entries = outcomes
        .into_iter()
        .map(outcome_value)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(json!({ "entries": entries }))
}
fn outcome_value(outcome: ConnectorOutcome) -> Result<Value, Box<HelperError>> {
    match outcome {
        ConnectorOutcome::Converted(converted) => {
            let request = serde_json::to_value(&converted.request)?;
            Ok(json!({ "name": converted.name, "request": request }))
        }
        ConnectorOutcome::Failed(failure) => {
            Ok(json!({ "name": failure.name, "error": failure.error }))
        }
    }
}
fn request_error(error: RequestError) -> Value {
    let message = match error {
        RequestError::Value(message) => message,
        RequestError::Shape(source) => source.to_string(),
    };
    json!({ "tag": "error", "message": message })
}
fn validation_error(error: &PayloadError) -> Value {
    let status = if matches!(error, PayloadError::UnknownRequiredFields { .. }) {
        500_u16
    } else {
        400_u16
    };
    json!({ "tag": "error", "status": status, "detail": { "error": error.to_string() } })
}

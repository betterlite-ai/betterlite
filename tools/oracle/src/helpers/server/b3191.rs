mod tests;

use betterlite_server::public_endpoints::agent_fields::catalog::{agent_fields, raw_agent_catalog};
use betterlite_server::public_endpoints::agent_fields::error::AgentFieldsError;
use betterlite_server::public_endpoints::agent_fields::types::AgentCreateInfo;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GetAgentFields,
    GetAgentField { agent_type: String },
    AgentCredentialFields { agent_type: String },
    AgentCatalogRaw,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fields_error = |error: AgentFieldsError| helper_error("AgentFieldsError", error);
        let json_error = |error: serde_json::Error| helper_error("SerializationError", error);
        let value = match input {
            Input::GetAgentFields => {
                let fields = agent_fields().map_err(fields_error)?;
                serde_json::to_value(fields).map_err(json_error)?
            }
            Input::GetAgentField { agent_type } => {
                let fields = agent_fields().map_err(fields_error)?;
                let agent = find_agent(&fields, agent_type)?;
                serde_json::to_value(agent).map_err(json_error)?
            }
            Input::AgentCredentialFields { agent_type } => {
                let fields = agent_fields().map_err(fields_error)?;
                let agent = find_agent(&fields, agent_type)?;
                serde_json::to_value(&agent.credential_fields).map_err(json_error)?
            }
            Input::AgentCatalogRaw => {
                let raw = raw_agent_catalog().map_err(fields_error)?;
                serde_json::to_value(raw).map_err(json_error)?
            }
        };
        Ok(value)
    }
}

fn find_agent<'fields>(
    fields: &'fields [AgentCreateInfo],
    agent_type: &str,
) -> Result<&'fields AgentCreateInfo, Box<HelperError>> {
    fields
        .iter()
        .find(|entry| entry.agent_type == agent_type)
        .ok_or_else(|| helper_error("AgentNotFound", format!("no agent type {agent_type}")))
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

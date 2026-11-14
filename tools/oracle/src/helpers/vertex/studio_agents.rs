use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_vertex::studio_agents::decode::{
    AgentOperation, AgentPayload, RawResponse, decode_studio_agent_delete,
    decode_studio_agent_payload,
};
use betterlite_vertex::studio_agents::error::StudioAgentError;
use betterlite_vertex::studio_agents::headers::validate_environment;
use betterlite_vertex::studio_agents::request::{AgentParams, BaseEnvironment, create_agent_body};
use betterlite_vertex::studio_agents::urls::{collection_target, versions_target};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    name: String,
    operation: Option<AgentOperation>,
    #[serde(default)]
    status: u16,
    #[serde(default)]
    body: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    base_agent: Option<String>,
    instructions: Option<String>,
    base_environment: Option<BaseEnvironment>,
    page_size: Option<i64>,
    page_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Validate,
    CreateBody,
    ListRequest,
    VersionsRequest,
    Reply,
    Delete,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> HelperResult {
    let params = AgentParams {
        api_key: call.api_key.as_deref().map(SecretString::from),
        api_base: call.api_base.clone(),
        base_agent: call.base_agent.clone(),
        instructions: call.instructions.clone(),
        base_environment: call.base_environment.clone(),
        page_size: call.page_size,
        page_token: call.page_token.clone(),
    };
    let variable = |name: &str| call.environment.get(name).filter(|value| !value.is_empty());
    let settings = Settings {
        environment_key: variable("GOOGLE_API_KEY")
            .or_else(|| variable("GEMINI_API_KEY"))
            .map(|key| SecretString::from(key.as_str())),
        generic_api_key: None,
        api_base: variable("GEMINI_API_BASE")
            .map_or_else(|| DEFAULT_API_BASE.to_owned(), Clone::clone),
    };
    let target = match call.function {
        Function::Validate => {
            let found = validate_environment(&call.headers, &params, &settings)?;
            let revealed = found
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret()))
                .collect::<BTreeMap<_, _>>();
            return Ok(json!({ "headers": revealed }));
        }
        Function::CreateBody => return Ok(json!({"body": create_agent_body(&call.name, &params)})),
        Function::ListRequest => collection_target(&params, &settings),
        Function::VersionsRequest => versions_target(&call.name, &params, &settings),
        Function::Reply | Function::Delete => return reply_value(call),
    };
    Ok(json!({ "url": target.url, "query": target.query }))
}

fn reply_value(call: &Call) -> HelperResult {
    let headers: IndexMap<_, _> = call.headers.clone().into_iter().collect();
    let response = RawResponse {
        status: call.status,
        headers: &headers,
        body: call.body.as_bytes(),
    };
    match (call.function, call.operation) {
        (Function::Delete, _) => Ok(json!(decode_studio_agent_delete(&response, &call.name)?)),
        (_, Some(operation)) => {
            let decoded = decode_studio_agent_payload(&response, operation, &call.name)?;
            Ok(match decoded.payload {
                AgentPayload::Agent(agent) => {
                    json!({ "agent": agent, "warnings": decoded.warnings })
                }
                AgentPayload::Agents(page) => json!(page),
                AgentPayload::Versions(page) => json!(page),
            })
        }
        (_, None) => Err("a reply names its operation").or_value_error(),
    }
}

impl From<StudioAgentError> for Box<HelperError> {
    fn from(error: StudioAgentError) -> Self {
        let status = match &error {
            StudioAgentError::Gemini { status, .. } => Some(*status),
            StudioAgentError::CustomBaseWithoutKey
            | StudioAgentError::MissingApiKey
            | StudioAgentError::ResponseTooLarge { .. }
            | StudioAgentError::Shape { .. } => None,
        };
        Self::new(HelperError {
            class: "ValueError".to_owned(),
            status,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

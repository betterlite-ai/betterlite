use std::collections::BTreeMap;

use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_models::decisions::Json;
use betterlite_vertex::studio_agents::decode::{
    AgentOperation, AgentPayload, RawResponse, decode_studio_agent_delete,
    decode_studio_agent_payload,
};
use betterlite_vertex::studio_agents::error::StudioAgentError;
use betterlite_vertex::studio_agents::headers::validate_environment;
use betterlite_vertex::studio_agents::records::{AgentPage, AgentVersionsPage, StudioAgent};
use betterlite_vertex::studio_agents::request::{AgentParams, BaseEnvironment, create_agent_body};
use betterlite_vertex::studio_agents::urls::{
    AgentTarget, collection_target, item_target, versions_target,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Validate(ValidateCall),
    CreateBody(CreateBodyCall),
    ListRequest(ListRequestCall),
    VersionsRequest(VersionsRequestCall),
    ItemRequest(ItemRequestCall),
    Reply(ReplyCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidateCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBodyCall {
    name: String,
    #[serde(default)]
    params: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequestCall {
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    page_size: Option<i64>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionsRequestCall {
    name: String,
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    page_size: Option<i64>,
    page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRequestCall {
    name: String,
    api_base: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    operation: ReplyOperation,
    name: String,
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplyOperation {
    Create,
    Get,
    List,
    Versions,
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
    match call {
        Call::Validate(call) => validate_value(call),
        Call::CreateBody(call) => create_body_value(call),
        Call::ListRequest(call) => list_request_value(call),
        Call::VersionsRequest(call) => versions_request_value(call),
        Call::ItemRequest(call) => item_request_value(call),
        Call::Reply(call) => reply_value(call),
    }
}

fn settings(environment: &BTreeMap<String, String>) -> Settings {
    Settings {
        environment_key: non_empty(environment, "GOOGLE_API_KEY")
            .or_else(|| non_empty(environment, "GEMINI_API_KEY"))
            .map(SecretString::from),
        generic_api_key: None,
        api_base: non_empty(environment, "GEMINI_API_BASE")
            .map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
    }
}

fn non_empty<'env>(environment: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    environment
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn validate_value(call: &ValidateCall) -> HelperResult {
    let params = AgentParams {
        api_key: call.api_key.as_deref().map(SecretString::from),
        api_base: call.api_base.clone(),
        base_agent: None,
        instructions: None,
        base_environment: None,
        page_size: None,
        page_token: None,
    };
    match validate_environment(&call.headers, &params, &settings(&call.environment)) {
        Ok(found) => {
            let revealed = found
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret()))
                .collect::<BTreeMap<_, _>>();
            Ok(json!({ "headers": revealed }))
        }
        Err(error) => Err(fail(error)),
    }
}

fn create_body_value(call: &CreateBodyCall) -> HelperResult {
    let params = AgentParams {
        api_key: None,
        api_base: None,
        base_agent: text_value(&call.params, "base_agent")?,
        instructions: text_value(&call.params, "instructions")?,
        base_environment: environment_value(&call.params)?,
        page_size: None,
        page_token: None,
    };
    Ok(json!({ "body": create_agent_body(&call.name, &params) }))
}

fn text_value(
    values: &BTreeMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, Box<HelperError>> {
    match values.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(&format!("{name} is not text"))),
    }
}

fn environment_value(
    values: &BTreeMap<String, Json>,
) -> Result<Option<BaseEnvironment>, Box<HelperError>> {
    match values.get("base_environment") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(BaseEnvironment::Id(text.clone()))),
        Some(Json::Object(fields)) => Ok(Some(BaseEnvironment::Object(fields.clone()))),
        Some(_) => Err(invalid("base_environment is neither an id nor an object")),
    }
}

fn request_params(
    api_base: Option<&str>,
    page_size: Option<i64>,
    page_token: Option<&str>,
) -> AgentParams {
    AgentParams {
        api_key: None,
        api_base: api_base.map(str::to_owned),
        base_agent: None,
        instructions: None,
        base_environment: None,
        page_size,
        page_token: page_token.map(str::to_owned),
    }
}

fn list_request_value(call: &ListRequestCall) -> HelperResult {
    let params = request_params(
        call.api_base.as_deref(),
        call.page_size,
        call.page_token.as_deref(),
    );
    let target = collection_target(&params, &settings(&call.environment));
    Ok(target_value(&target))
}

fn versions_request_value(call: &VersionsRequestCall) -> HelperResult {
    let params = request_params(
        call.api_base.as_deref(),
        call.page_size,
        call.page_token.as_deref(),
    );
    let target = versions_target(&call.name, &params, &settings(&call.environment));
    Ok(target_value(&target))
}

fn item_request_value(call: &ItemRequestCall) -> HelperResult {
    let params = request_params(call.api_base.as_deref(), None, None);
    let url = item_target(&call.name, &params, &settings(&call.environment)).url;
    Ok(json!({ "url": url.clone(), "delete_url": url, "query": [] }))
}

fn target_value(target: &AgentTarget) -> Value {
    let query: Vec<[&str; 2]> = target
        .query
        .iter()
        .map(|(name, value)| [name.as_str(), value.as_str()])
        .collect();
    json!({ "url": target.url, "query": query })
}

fn reply_value(call: &ReplyCall) -> HelperResult {
    let carried = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<IndexMap<_, _>>();
    let response = RawResponse {
        status: call.status,
        headers: &carried,
        body: call.body.as_bytes(),
    };
    if let ReplyOperation::Delete = call.operation {
        let deleted = decode_studio_agent_delete(&response, &call.name).map_err(fail)?;
        return Ok(json!({ "name": deleted.name, "deleted": deleted.deleted }));
    }
    let payload = payload_operation(call.operation)
        .ok_or_else(|| invalid("the reply operation is not a JSON payload operation"))?;
    let decoded = decode_studio_agent_payload(&response, payload, &call.name).map_err(fail)?;
    let warnings = decoded
        .warnings
        .iter()
        .map(|warning| warning.subject())
        .collect::<Vec<_>>();
    match decoded.payload {
        AgentPayload::Agent(agent) => Ok(agent_value(&agent, &warnings)),
        AgentPayload::Agents(page) => Ok(page_value(&page)),
        AgentPayload::Versions(page) => Ok(versions_value(&page)),
    }
}

const fn payload_operation(operation: ReplyOperation) -> Option<AgentOperation> {
    match operation {
        ReplyOperation::Create => Some(AgentOperation::Create),
        ReplyOperation::Get => Some(AgentOperation::Get),
        ReplyOperation::List => Some(AgentOperation::List),
        ReplyOperation::Versions => Some(AgentOperation::Versions),
        ReplyOperation::Delete => None,
    }
}

fn agent_value(agent: &StudioAgent, warnings: &[&str]) -> Value {
    json!({
        "agent": {
            "id": agent.id,
            "name": agent.name,
            "extension": agent.extension,
        },
        "warnings": warnings,
    })
}

fn page_value(page: &AgentPage) -> Value {
    json!({ "agents": page.agents, "next_page_token": page.next_page_token })
}

fn versions_value(page: &AgentVersionsPage) -> Value {
    json!({ "agent_versions": page.agent_versions, "next_page_token": page.next_page_token })
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn fail(error: StudioAgentError) -> Box<HelperError> {
    let status = match &error {
        StudioAgentError::Gemini { status, .. } => Some(*status),
        _ => None,
    };
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

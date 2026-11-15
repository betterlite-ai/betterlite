use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::error::StudioAgentError;
use super::records::{
    AgentDeleteResult, AgentPage, AgentVersionsPage, AgentWarning, NativeObject, StudioAgent,
};

pub const RESPONSE_BODY_LIMIT: usize = 0x0040_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperation {
    Create,
    Get,
    List,
    Versions,
}

#[derive(Debug, Clone, Copy)]
pub struct RawResponse<'raw> {
    pub status: u16,
    pub headers: &'raw IndexMap<String, String>,
    pub body: &'raw [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPayload {
    Agent(StudioAgent),
    Agents(AgentPage),
    Versions(AgentVersionsPage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPayload {
    pub payload: AgentPayload,
    pub warnings: Vec<AgentWarning>,
}

pub fn decode_studio_agent_payload(
    response: &RawResponse<'_>,
    operation: AgentOperation,
    request_name: &str,
) -> Result<DecodedPayload, StudioAgentError> {
    if response.body.len() > RESPONSE_BODY_LIMIT {
        return Err(StudioAgentError::ResponseTooLarge {
            size: response.body.len(),
            limit: RESPONSE_BODY_LIMIT,
        });
    }
    ensure_success(response)?;
    let parsed = serde_json::from_slice::<Json>(response.body).ok();
    let mut warnings = Vec::new();
    let payload = match operation {
        AgentOperation::Create | AgentOperation::Get => {
            if parsed.is_none() && operation == AgentOperation::Create {
                tracing::warn!("non-JSON create response; the request name is used");
                warnings.push(AgentWarning::MalformedCreateReply);
            }
            AgentPayload::Agent(agent_record(parsed, request_name)?)
        }
        AgentOperation::List => {
            let (agents, next_page_token) = page(parsed, "agents")?;
            AgentPayload::Agents(AgentPage {
                agents,
                next_page_token,
            })
        }
        AgentOperation::Versions => {
            let (agent_versions, next_page_token) = page(parsed, "agentVersions")?;
            AgentPayload::Versions(AgentVersionsPage {
                agent_versions,
                next_page_token,
            })
        }
    };
    Ok(DecodedPayload { payload, warnings })
}

pub fn decode_studio_agent_delete(
    response: &RawResponse<'_>,
    request_name: &str,
) -> Result<AgentDeleteResult, StudioAgentError> {
    ensure_success(response)?;
    Ok(AgentDeleteResult {
        name: request_name.to_owned(),
        deleted: true,
    })
}

fn ensure_success(response: &RawResponse<'_>) -> Result<(), StudioAgentError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    Err(StudioAgentError::Gemini {
        status: response.status,
        body: str::from_utf8(response.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", response.body.len()),
            str::to_owned,
        ),
        headers: response.headers.clone(),
    })
}

fn object(parsed: Option<Json>) -> Result<NativeObject, StudioAgentError> {
    match parsed {
        None => Ok(NativeObject::new()),
        Some(Json::Object(fields)) => Ok(fields),
        Some(_) => Err(StudioAgentError::Shape { field: "root" }),
    }
}

fn agent_record(parsed: Option<Json>, request_name: &str) -> Result<StudioAgent, StudioAgentError> {
    let mut extension = object(parsed)?;
    let id = match extension.shift_remove("id") {
        None => Some(request_name.to_owned()),
        Some(value) => optional_text("id", value)?,
    };
    let name = match extension.shift_remove("name") {
        None => id.clone(),
        Some(value) => optional_text("name", value)?,
    };
    Ok(StudioAgent {
        id,
        name,
        extension,
    })
}

fn optional_text(field: &'static str, value: Json) -> Result<Option<String>, StudioAgentError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text)),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(StudioAgentError::Shape { field })
        }
    }
}

fn page(
    parsed: Option<Json>,
    field: &'static str,
) -> Result<(Vec<NativeObject>, Option<String>), StudioAgentError> {
    let mut fields = object(parsed)?;
    let items = match fields.shift_remove(field) {
        None => Vec::new(),
        Some(Json::List(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                Json::Object(item) => Ok(item),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    Err(StudioAgentError::Shape { field })
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(StudioAgentError::Shape { field }),
    };
    let token = fields
        .shift_remove("nextPageToken")
        .map_or(Ok(None), |value| optional_text("nextPageToken", value))?;
    Ok((items, token))
}

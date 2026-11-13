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

// The status decides before any body is read, as _raise_for_status runs first in the source.
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
    if !(200..300).contains(&response.status) {
        return Err(gemini_error(response));
    }
    match operation {
        AgentOperation::Create | AgentOperation::Get => {
            agent_payload(response.body, operation, request_name)
        }
        AgentOperation::List | AgentOperation::Versions => page_payload(response.body, operation),
    }
}

fn agent_payload(
    body: &[u8],
    operation: AgentOperation,
    request_name: &str,
) -> Result<DecodedPayload, StudioAgentError> {
    let (agent, malformed) = agent_record(body, request_name)?;
    let mut warnings = Vec::new();
    if malformed && operation == AgentOperation::Create {
        tracing::warn!(
            "Gemini answered the create-agent call with a non-JSON body; the request name is used"
        );
        warnings.push(AgentWarning::MalformedCreateReply);
    }
    Ok(DecodedPayload {
        payload: AgentPayload::Agent(agent),
        warnings,
    })
}

fn page_payload(
    body: &[u8],
    operation: AgentOperation,
) -> Result<DecodedPayload, StudioAgentError> {
    let payload = if operation == AgentOperation::List {
        let (agents, next_page_token) = page(body, "agents")?;
        AgentPayload::Agents(AgentPage {
            agents,
            next_page_token,
        })
    } else {
        let (agent_versions, next_page_token) = page(body, "agentVersions")?;
        AgentPayload::Versions(AgentVersionsPage {
            agent_versions,
            next_page_token,
        })
    };
    Ok(DecodedPayload {
        payload,
        warnings: Vec::new(),
    })
}

// Gemini answers a delete with an empty body; the requested name is the result either way.
pub fn decode_studio_agent_delete(
    response: &RawResponse<'_>,
    request_name: &str,
) -> Result<AgentDeleteResult, StudioAgentError> {
    if !(200..300).contains(&response.status) {
        return Err(gemini_error(response));
    }
    Ok(AgentDeleteResult {
        name: request_name.to_owned(),
        deleted: true,
    })
}

// A non-JSON body falls back to the request name; valid JSON that is not an object still fails.
fn agent_record(body: &[u8], request_name: &str) -> Result<(StudioAgent, bool), StudioAgentError> {
    let Some(value) = serde_json::from_slice::<Json>(body).ok() else {
        let agent = StudioAgent {
            id: Some(request_name.to_owned()),
            name: Some(request_name.to_owned()),
            extension: NativeObject::new(),
        };
        return Ok((agent, true));
    };
    let Json::Object(fields) = value else {
        return Err(StudioAgentError::NotAnObject);
    };
    let id = match fields.get("id") {
        None => Some(request_name.to_owned()),
        Some(value) => optional_text("id", value)?,
    };
    let name = match fields.get("name") {
        None => id.clone(),
        Some(value) => optional_text("name", value)?,
    };
    let mut extension = fields;
    extension.shift_remove("id");
    extension.shift_remove("name");
    Ok((
        StudioAgent {
            id,
            name,
            extension,
        },
        false,
    ))
}

fn optional_text(field: &'static str, value: &Json) -> Result<Option<String>, StudioAgentError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(StudioAgentError::NotText { field })
        }
    }
}

// A non-JSON body is an empty page; a list field that is not a list of objects still fails.
fn page(
    body: &[u8],
    field: &'static str,
) -> Result<(Vec<NativeObject>, Option<String>), StudioAgentError> {
    let Some(value) = serde_json::from_slice::<Json>(body).ok() else {
        return Ok((Vec::new(), None));
    };
    let Json::Object(mut fields) = value else {
        return Err(StudioAgentError::NotAnObject);
    };
    let items = match fields.shift_remove(field) {
        None => Vec::new(),
        Some(Json::List(entries)) => entries
            .into_iter()
            .map(|entry| match entry {
                Json::Object(fields) => Ok(fields),
                _ => Err(StudioAgentError::NotAnObjectList { field }),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(StudioAgentError::NotAnObjectList { field }),
    };
    let token = match fields.shift_remove("nextPageToken") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text),
        Some(_) => {
            return Err(StudioAgentError::NotText {
                field: "nextPageToken",
            });
        }
    };
    Ok((items, token))
}

fn gemini_error(response: &RawResponse<'_>) -> StudioAgentError {
    StudioAgentError::Gemini {
        status: response.status,
        body: str::from_utf8(response.body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", response.body.len()),
            str::to_owned,
        ),
        headers: response.headers.clone(),
    }
}

use betterlite_bedrock::agentcore_response::transform::RawResponse;
use betterlite_bedrock::invoke_agent_reply::error::InvokeAgentReplyError;
use betterlite_bedrock::invoke_agent_reply::events::{AgentEvent, EventHeaders};
use betterlite_bedrock::invoke_agent_reply::extract::response_content;
use betterlite_bedrock::invoke_agent_reply::reply::transform_response;
use betterlite_bedrock::invoke_agent_reply::wire::ModelResponse;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    transform: Option<Vec<TransformCase>>,
    content: Option<Vec<ContentCase>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformCase {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    supplied: ModelResponse,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentCase {
    events: Vec<EventCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventCase {
    event_type: String,
    content_type: String,
    message_type: String,
    payload: Json,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match (&input.transform, &input.content) {
            (Some(cases), None) => cases
                .iter()
                .map(transform_case)
                .collect::<Result<Vec<_>, _>>()
                .map(|views| json!(views)),
            (None, Some(cases)) => Ok(json!(cases.iter().map(content_case).collect::<Vec<_>>())),
            (Some(_), Some(_)) | (None, None) => Err(failure(
                "invoke_agent_reply takes exactly one of transform and content".to_owned(),
            )),
        }
    }
}

fn transform_case(case: &TransformCase) -> Result<Value, Box<HelperError>> {
    let body = hex::decode(&case.body).map_err(|error| failure(error.to_string()))?;
    let raw = RawResponse {
        status_code: case.status,
        headers: case.headers.clone(),
        body,
    };
    let mut response = case.supplied.clone();
    match transform_response(&raw, &mut response) {
        Ok(()) => Ok(json!({ "response": response })),
        Err(InvokeAgentReplyError::Bedrock { status, .. }) => {
            Ok(json!({ "failure": { "status": status } }))
        }
        Err(error) => Err(failure(error.to_string())),
    }
}

fn content_case(case: &ContentCase) -> Value {
    let events = case.events.iter().map(event_of).collect::<Vec<_>>();
    match response_content(&events) {
        Ok(content) => json!({ "content": content }),
        Err(_) => json!({ "failure": "structural" }),
    }
}

fn event_of(case: &EventCase) -> AgentEvent {
    AgentEvent {
        headers: EventHeaders {
            event_type: case.event_type.clone(),
            content_type: case.content_type.clone(),
            message_type: case.message_type.clone(),
        },
        payload: case.payload.clone(),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvokeAgentReply".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

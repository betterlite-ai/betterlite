use std::fmt::Display;

use betterlite_bedrock::realtime_session::records::{
    MediaType, PromptStart, SessionEvent, SessionEventType,
};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

#[derive(Serialize)]
struct EventSummary {
    event_id: String,
    #[serde(rename = "type")]
    event_type: &'static str,
    session: SessionSummary,
}

#[derive(Serialize)]
struct SessionSummary {
    id: String,
    modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Serialize)]
struct PromptStartSummary {
    prompt_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use_output_configuration: Option<MediaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolSummary>>,
}

#[derive(Serialize)]
struct ToolSummary {
    name: String,
    description: String,
    schema: Value,
}

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RealtimeSession".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn parse_ids(values: &[String]) -> Result<Vec<Uuid>, Box<HelperError>> {
    values
        .iter()
        .map(Uuid::parse_str)
        .collect::<Result<Vec<Uuid>, _>>()
        .map_err(failure)
}

pub(super) fn header_pair(header: &[String]) -> Result<(String, String), Box<HelperError>> {
    match header {
        [name, value] => Ok((name.clone(), value.clone())),
        _ => Err(failure("a header is one name and one value")),
    }
}

pub(super) fn prompt_start_summary(prompt_start: &PromptStart) -> HelperResult {
    let tools = match &prompt_start.tool_configuration {
        Some(configuration) => {
            let mut summaries: Vec<ToolSummary> = Vec::new();
            for tool in &configuration.tools {
                let schema = serde_json::from_str::<Value>(&tool.tool_spec.input_schema.json)
                    .map_err(failure)?;
                summaries.push(ToolSummary {
                    name: tool.tool_spec.name.clone(),
                    description: tool.tool_spec.description.clone(),
                    schema,
                });
            }
            Some(summaries)
        }
        None => None,
    };
    serde_json::to_value(PromptStartSummary {
        prompt_name: prompt_start.prompt_name.clone(),
        tool_use_output_configuration: prompt_start.tool_use_output_configuration.clone(),
        tools,
    })
    .map_err(failure)
}

pub(super) fn event_summary(event: &SessionEvent) -> HelperResult {
    let event_type = match event.event_type {
        SessionEventType::Created => "session.created",
        SessionEventType::Updated => "session.updated",
    };
    serde_json::to_value(EventSummary {
        event_id: event.event_id.clone(),
        event_type,
        session: SessionSummary {
            id: event.session.id.clone(),
            modalities: event.session.modalities.clone(),
            model: event.session.model.clone(),
        },
    })
    .map_err(failure)
}

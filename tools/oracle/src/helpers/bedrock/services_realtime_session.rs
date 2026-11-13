use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::invoke_primitives::environment::error_class;
use betterlite_bedrock::realtime_session::records::{
    MediaType, PromptStart, SessionEvent, SessionEventType, UpdateFrame,
};
use betterlite_bedrock::realtime_session::update::SessionUpdate;
use betterlite_bedrock::realtime_session::{
    REQUIRES_SESSION_CONFIGURATION, RealtimeSession, complete_url,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    case: Case,
    ids: Vec<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    updates: Vec<SessionUpdate>,
    #[serde(default)]
    urls: Vec<Option<String>>,
    #[serde(default)]
    modalities: Option<Vec<String>>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    headers: Vec<Vec<String>>,
    #[serde(default)]
    error: Option<ErrorRun>,
}

#[derive(Debug, Deserialize)]
enum Case {
    #[serde(rename = "default_session_configuration")]
    DefaultSessionConfiguration,
    #[serde(rename = "session_update_options")]
    SessionUpdateOptions,
    #[serde(rename = "pcm16_input_output_rates")]
    Pcm16InputOutputRates,
    #[serde(rename = "g711_rates")]
    G711Rates,
    #[serde(rename = "explicit_sample_rate")]
    ExplicitSampleRate,
    #[serde(rename = "tool_configuration")]
    ToolConfiguration,
    #[serde(rename = "system_instructions")]
    SystemInstructions,
    #[serde(rename = "created_updated_events")]
    CreatedUpdatedEvents,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorRun {
    message: String,
    status: u16,
    headers: Vec<Vec<String>>,
}

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

pub(crate) struct Op;

pub(crate) struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    pub(crate) fn new(values: &[Uuid]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

fn failure<E>(error: E) -> Box<HelperError>
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

fn parse_ids(values: &[String]) -> Result<Vec<Uuid>, Box<HelperError>> {
    values
        .iter()
        .map(Uuid::parse_str)
        .collect::<Result<Vec<Uuid>, _>>()
        .map_err(failure)
}

fn update_values(input: &Input, ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let mut session = RealtimeSession::new(&ids);
    let mut results: Vec<Value> = Vec::new();
    for update in &input.updates {
        let frames = session.apply_update(update, &ids);
        let value = serde_json::to_value(&frames).map_err(failure)?;
        results.push(json!({ "frames": value }));
    }
    Ok(json!({
        "updates": results,
        "prompt_name": session.names.prompt,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
    }))
}

fn prompt_start_summary(prompt_start: &PromptStart) -> HelperResult {
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

fn tool_values(input: &Input, ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let mut session = RealtimeSession::new(&ids);
    let mut results: Vec<Value> = Vec::new();
    for update in &input.updates {
        let frames = session.apply_update(update, &ids);
        let prompt_start = match frames.get(1) {
            Some(UpdateFrame::PromptStart(frame)) => &frame.event.prompt_start,
            _ => return Err(failure("the second frame is promptStart")),
        };
        let summary = prompt_start_summary(prompt_start)?;
        results.push(json!({ "prompt_start": summary }));
    }
    Ok(json!({
        "updates": results,
        "prompt_name": session.names.prompt,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
    }))
}

fn event_summary(event: &SessionEvent) -> HelperResult {
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

fn header_pair(header: &[String]) -> Result<(String, String), Box<HelperError>> {
    match header {
        [name, value] => Ok((name.clone(), value.clone())),
        _ => Err(failure("a header is one name and one value")),
    }
}

fn default_case(ids: &[Uuid]) -> HelperResult {
    let ids = FixedIds::new(ids);
    let session = RealtimeSession::new(&ids);
    let configuration = session.configuration_request(None);
    Ok(json!({
        "prompt_name": session.names.prompt,
        "content_name": session.names.content,
        "audio_content_name": session.names.audio_content,
        "prompt_started": session.prompt_started,
        "client_audio_streamed": session.client_audio_streamed,
        "configuration": serde_json::to_value(&configuration).map_err(failure)?,
        "prompt_started_after": session.prompt_started,
    }))
}

fn created_updated(input: &Input) -> HelperResult {
    let ids = FixedIds::new(&parse_ids(&input.ids)?);
    let created =
        RealtimeSession::session_created_event(Some(input.model.as_str()), &input.trace_id, &ids);
    let updated_default = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        None,
        &ids,
    );
    let no_modalities: &[String] = &[];
    let updated_empty = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        Some(no_modalities),
        &ids,
    );
    let updated_explicit = RealtimeSession::session_updated_event(
        Some(input.model.as_str()),
        &input.trace_id,
        input.modalities.as_deref(),
        &ids,
    );
    let model_name = input.models.first().map_or("", String::as_str);
    let model_event =
        RealtimeSession::session_created_event(Some(model_name), &input.trace_id, &ids);
    let urls: Vec<String> = input
        .urls
        .iter()
        .map(|url| complete_url(url.as_deref()))
        .collect();
    let mut validated: Vec<Value> = Vec::new();
    for header in &input.headers {
        let pair = header_pair(header)?;
        validated.push(json!([pair.0, pair.1]));
    }
    let error = match &input.error {
        Some(error) => {
            let mut pairs: Vec<(String, String)> = Vec::new();
            for header in &error.headers {
                pairs.push(header_pair(header)?);
            }
            let built = error_class(error.message.clone(), error.status, pairs).map_err(failure)?;
            (
                built.error.status_code,
                built.error.status_code_is_synthesized,
            )
        }
        None => (0, false),
    };
    Ok(json!({
        "created": event_summary(&created)?,
        "updated_default": event_summary(&updated_default)?,
        "updated_empty": event_summary(&updated_empty)?,
        "updated_explicit": event_summary(&updated_explicit)?,
        "model_event": event_summary(&model_event)?,
        "requires_session_configuration": REQUIRES_SESSION_CONFIGURATION,
        "complete_urls": urls,
        "validated_headers": validated,
        "error_status": error.0,
        "error_synthesized": error.1,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let ids = parse_ids(&input.ids)?;
        match input.case {
            Case::DefaultSessionConfiguration => default_case(&ids),
            Case::CreatedUpdatedEvents => created_updated(input),
            Case::ToolConfiguration => tool_values(input, &ids),
            Case::SessionUpdateOptions
            | Case::Pcm16InputOutputRates
            | Case::G711Rates
            | Case::ExplicitSampleRate
            | Case::SystemInstructions => update_values(input, &ids),
        }
    }
}

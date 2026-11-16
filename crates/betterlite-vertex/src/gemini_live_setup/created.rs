use serde::Serialize;

use super::decode::decode_gemini_live_setup;
use super::error::LiveSetupError;
use super::wire::{Instruction, LiveSetup};

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct SessionCreatedEvent {
    #[serde(rename = "type")]
    pub event_type: CreatedEvent,
    pub session: CreatedSession,
    pub event_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum CreatedEvent {
    #[serde(rename = "session.created")]
    SessionCreated,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct CreatedSession {
    pub id: String,
    pub modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub model: String,
}

pub fn session_created(
    model: &str,
    logging_session_id: &str,
    session_configuration_request: Option<&str>,
    event_id: &str,
) -> Result<SessionCreatedEvent, LiveSetupError> {
    let setup = match session_configuration_request {
        Some(request) if !request.is_empty() => decode_gemini_live_setup(request)?,
        Some(_) | None => LiveSetup::default(),
    };
    let modalities = setup
        .generation_config
        .response_modalities
        .unwrap_or_else(|| vec!["AUDIO".to_owned()])
        .into_iter()
        .map(|modality| modality.to_lowercase())
        .collect();
    let instructions = match setup.system_instruction {
        Some(Instruction::Text(text)) => Some(text),
        Some(Instruction::Content(_)) | None => None,
    };
    let resolved_model = setup.model.unwrap_or_else(|| model.to_owned());
    Ok(SessionCreatedEvent {
        event_type: CreatedEvent::SessionCreated,
        session: CreatedSession {
            id: logging_session_id.to_owned(),
            modalities,
            instructions,
            model: bare_model(&resolved_model).to_owned(),
        },
        event_id: event_id.to_owned(),
    })
}

fn bare_model(model: &str) -> &str {
    if let Some((_prefix, bare)) = model.rsplit_once("/models/") {
        return bare;
    }
    model.strip_prefix("models/").unwrap_or(model)
}

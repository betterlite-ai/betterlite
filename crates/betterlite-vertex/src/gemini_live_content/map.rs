use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;
use serde::Serialize;

use super::error::ContentError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum MappedEvent {
    #[serde(rename = "session.created")]
    SessionCreated,
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone,
    #[serde(rename = "response.text.delta")]
    TextDelta,
    #[serde(rename = "response.audio.delta")]
    AudioDelta,
    #[serde(rename = "response.text.done")]
    TextDone,
    #[serde(rename = "response.audio.done")]
    AudioDone,
    #[serde(rename = "response.done")]
    ResponseDone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
pub enum ModalityEvent {
    #[serde(rename = "response.text.delta")]
    TextDelta,
    #[serde(rename = "response.audio.delta")]
    AudioDelta,
    #[serde(rename = "response.text.done")]
    TextDone,
    #[serde(rename = "response.audio.done")]
    AudioDone,
}

impl ModalityEvent {
    #[must_use]
    pub const fn delta_type(self) -> AllDeltaType {
        match self {
            Self::TextDelta | Self::TextDone => AllDeltaType::Text,
            Self::AudioDelta | Self::AudioDone => AllDeltaType::Audio,
        }
    }

    #[must_use]
    pub const fn from_mapped(event: MappedEvent) -> Option<Self> {
        match event {
            MappedEvent::TextDelta => Some(Self::TextDelta),
            MappedEvent::AudioDelta => Some(Self::AudioDelta),
            MappedEvent::TextDone => Some(Self::TextDone),
            MappedEvent::AudioDone => Some(Self::AudioDone),
            MappedEvent::SessionCreated
            | MappedEvent::FunctionCallArgumentsDone
            | MappedEvent::ResponseDone => None,
        }
    }
}

struct FieldRule {
    map_key: &'static str,
    nested: Option<&'static str>,
    event: MappedEvent,
}

const FIELD_RULES: [FieldRule; 5] = [
    FieldRule {
        map_key: "setupComplete",
        nested: None,
        event: MappedEvent::SessionCreated,
    },
    FieldRule {
        map_key: "serverContent.generationComplete",
        nested: Some("generationComplete"),
        event: MappedEvent::TextDone,
    },
    FieldRule {
        map_key: "serverContent.turnComplete",
        nested: Some("turnComplete"),
        event: MappedEvent::ResponseDone,
    },
    FieldRule {
        map_key: "serverContent.interrupted",
        nested: Some("interrupted"),
        event: MappedEvent::ResponseDone,
    },
    FieldRule {
        map_key: "toolCall",
        nested: None,
        event: MappedEvent::FunctionCallArgumentsDone,
    },
];

#[must_use]
pub fn get_nested_value<'value>(value: &'value Json, path: &str) -> Option<&'value Json> {
    let mut current = value;
    for key in path.split('.') {
        let fields = current.as_object()?;
        current = fields.get(key)?;
    }
    Some(current)
}

pub fn map_model_turn_event(model_turn: &Json) -> Result<MappedEvent, ContentError> {
    let fields = model_turn
        .as_object()
        .ok_or(ContentError::ModelTurnNotObject)?;
    let Some(parts) = fields.get("parts") else {
        return Err(ContentError::MissingParts);
    };
    let Json::List(parts) = parts else {
        return Err(ContentError::PartsNotList);
    };
    if parts.len() != 1 {
        tracing::warn!(
            "Realtime: Expected 1 part, got {} for Gemini model turn event.",
            parts.len()
        );
    }
    let Some(part) = parts.first() else {
        return Err(ContentError::EmptyParts);
    };
    let part = part.as_object().ok_or(ContentError::PartNotObject)?;
    if part.contains_key("text") {
        return Ok(MappedEvent::TextDelta);
    }
    if part.contains_key("inlineData") {
        return Ok(MappedEvent::AudioDelta);
    }
    Err(ContentError::UnexpectedPart)
}

pub const fn map_generation_complete_event(
    delta_type: Option<AllDeltaType>,
) -> Result<MappedEvent, ContentError> {
    match delta_type {
        Some(AllDeltaType::Text) => Ok(MappedEvent::TextDone),
        Some(AllDeltaType::Audio) => Ok(MappedEvent::AudioDone),
        None => Err(ContentError::MissingDeltaType),
    }
}

pub fn map_openai_event(
    key: &str,
    value: &Json,
    delta_type: Option<AllDeltaType>,
) -> Result<MappedEvent, ContentError> {
    let fields = value.as_object();
    if let Some(model_turn) = fields
        .and_then(|fields| fields.get("modelTurn"))
        .filter(|model_turn| model_turn.is_truthy())
    {
        return map_model_turn_event(model_turn);
    }
    if fields
        .and_then(|fields| fields.get("generationComplete"))
        .is_some_and(Json::is_truthy)
    {
        return map_generation_complete_event(delta_type);
    }
    for rule in FIELD_RULES {
        if rule.map_key == key {
            return Ok(rule.event);
        }
        if let Some(path) = rule.nested
            && rule
                .map_key
                .split_once('.')
                .is_some_and(|(prefix, _)| prefix == key)
            && get_nested_value(value, path).is_some_and(|found| !matches!(found, Json::Null))
        {
            return Ok(rule.event);
        }
    }
    Err(ContentError::UnknownEvent(key.to_owned()))
}

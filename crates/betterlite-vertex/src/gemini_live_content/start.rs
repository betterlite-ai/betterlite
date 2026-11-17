use betterlite_core::api::realtime_contracts::events::AllDeltaType;

use super::error::ContentError;
use super::events::{
    ContentEvent, ContentPart, ContentPartAdded, ConversationItemAdded, OutputItem,
    OutputItemAdded, ResponseCreated, ResponseObject,
};
use super::ids::ContentIds;
use crate::gemini_live_setup::decode::decode_gemini_live_setup;
use crate::gemini_live_setup::error::LiveSetupError;
use crate::gemini_live_setup::wire::GenerationConfig;

pub struct StartContent<'content> {
    pub setup_request: Option<&'content str>,
    pub response_id: &'content str,
    pub output_item_id: &'content str,
    pub conversation_id: &'content str,
    pub delta_type: AllDeltaType,
}

pub fn start_content_events(
    content: &StartContent<'_>,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let generation = generation_config(content.setup_request)?;
    let modalities = lowered_modalities(&generation);
    let (part, item) = added_content(content.delta_type, content.output_item_id);
    let mut events = vec![response_created(content, generation, modalities, ids)];
    events.extend(added_events(content, &item, part, ids));
    Ok(events)
}

#[must_use]
fn added_events(
    content: &StartContent<'_>,
    item: &OutputItem,
    part: ContentPart,
    ids: &mut dyn ContentIds,
) -> [ContentEvent; 3] {
    [
        ContentEvent::OutputItemAdded(OutputItemAdded {
            event_id: format!("event_{}", ids.next_id()),
            response_id: content.response_id.to_owned(),
            output_index: 0,
            item: item.clone(),
        }),
        ContentEvent::ConversationItemAdded(ConversationItemAdded {
            event_id: format!("event_{}", ids.next_id()),
            previous_item_id: None,
            item: item.clone(),
        }),
        ContentEvent::ContentPartAdded(ContentPartAdded {
            content_index: 0,
            output_index: 0,
            event_id: format!("event_{}", ids.next_id()),
            item_id: content.output_item_id.to_owned(),
            part,
            response_id: content.response_id.to_owned(),
        }),
    ]
}

fn lowered_modalities(generation: &GenerationConfig) -> Vec<String> {
    generation
        .response_modalities
        .clone()
        .unwrap_or_else(|| vec!["AUDIO".to_owned()])
        .into_iter()
        .map(|modality| modality.to_lowercase())
        .collect()
}

fn added_content(delta_type: AllDeltaType, output_item_id: &str) -> (ContentPart, OutputItem) {
    let part = match delta_type {
        AllDeltaType::Text => ContentPart::Text {
            text: String::new(),
        },
        AllDeltaType::Audio => ContentPart::Audio {
            transcript: String::new(),
        },
    };
    let item = OutputItem {
        id: output_item_id.to_owned(),
        object: "realtime.item".to_owned(),
        item_type: "message".to_owned(),
        status: "in_progress".to_owned(),
        role: "assistant".to_owned(),
        content: Vec::new(),
    };
    (part, item)
}

#[must_use]
fn response_created(
    content: &StartContent<'_>,
    generation: GenerationConfig,
    modalities: Vec<String>,
    ids: &mut dyn ContentIds,
) -> ContentEvent {
    ContentEvent::ResponseCreated(ResponseCreated {
        event_id: format!("event_{}", ids.next_id()),
        response: ResponseObject {
            object: "realtime.response".to_owned(),
            id: content.response_id.to_owned(),
            status: "in_progress".to_owned(),
            status_details: None,
            output: Vec::new(),
            conversation_id: content.conversation_id.to_owned(),
            modalities,
            temperature: generation.temperature,
            max_output_tokens: generation.max_output_tokens,
        },
    })
}

fn generation_config(request: Option<&str>) -> Result<GenerationConfig, ContentError> {
    let Some(raw) = request else {
        return Ok(GenerationConfig::default());
    };
    match decode_gemini_live_setup(raw) {
        Ok(setup) => Ok(setup.generation_config),
        Err(LiveSetupError::InvalidJson(_source)) => Ok(GenerationConfig::default()),
        Err(error) => Err(ContentError::Setup(error)),
    }
}

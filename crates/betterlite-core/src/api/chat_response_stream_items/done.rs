use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartDoneEvent, ContentPartDonePart, Extra, OutputItemDoneEvent, OutputTextDoneEvent,
    ReasoningSummaryPartDoneEvent, ReasoningSummaryTextDoneEvent, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use indexmap::IndexMap;

use crate::api::chat_response_stream_tools::state::ChatStreamState;

use super::completion::{completion_annotations, completion_content};
use super::error::StreamItemsError;
use super::items::ensure_cached_item_id;
use super::sources::EventSources;

#[derive(Clone, Copy)]
pub enum MessageDone {
    Text,
    ContentPart,
    Item,
}

pub fn message_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
    kind: MessageDone,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    let output_index = state.progress.message_output_index;
    match kind {
        MessageDone::Text => {
            let text = completion_content(completion)?;
            Ok(KnownResponseEvent::OutputTextDone(OutputTextDoneEvent {
                item_id,
                output_index,
                content_index: 0,
                text: text.to_owned(),
                extra: Extra::new(),
            }))
        }
        MessageDone::ContentPart => {
            let part = done_part(completion)?;
            Ok(KnownResponseEvent::ContentPartDone(ContentPartDoneEvent {
                item_id,
                output_index,
                content_index: 0,
                part,
                extra: Extra::new(),
            }))
        }
        MessageDone::Item => message_item_done(state, &item_id),
    }
}

// LiteLLM's builder ignores its completion argument here and reads the stored response instead.
fn message_item_done<Context>(
    state: &ChatStreamState<Context>,
    item_id: &str,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let Some(stored) = &state.progress.litellm_model_response else {
        return Err(StreamItemsError::MissingModelResponse);
    };
    let text = completion_content(stored)?;
    let annotations = completion_annotations(stored)?;
    Ok(KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
        output_index: state.progress.message_output_index,
        sequence_number: 1,
        item: completed_message_item(item_id, text, &annotations),
        extra: Extra::new(),
    }))
}

#[derive(Clone, Copy)]
pub enum ReasoningDone {
    Text,
    Part,
    Item,
}

#[must_use]
pub fn reasoning_done_event(
    item_id: &str,
    content: &str,
    sequence_number: i64,
    kind: ReasoningDone,
) -> KnownResponseEvent {
    match kind {
        ReasoningDone::Text => {
            KnownResponseEvent::ReasoningSummaryTextDone(ReasoningSummaryTextDoneEvent {
                item_id: item_id.to_owned(),
                output_index: 0,
                sequence_number,
                summary_index: 0,
                text: content.to_owned(),
                extra: Extra::new(),
            })
        }
        ReasoningDone::Part => {
            KnownResponseEvent::ReasoningSummaryPartDone(ReasoningSummaryPartDoneEvent {
                item_id: item_id.to_owned(),
                output_index: 0,
                sequence_number,
                summary_index: 0,
                part: summary_part(content),
                extra: Extra::new(),
            })
        }
        ReasoningDone::Item => KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
            output_index: 0,
            sequence_number,
            item: reasoning_item(item_id, content),
            extra: Extra::new(),
        }),
    }
}

fn done_part(completion: &Json) -> Result<ContentPartDonePart, StreamItemsError> {
    Ok(ContentPartDonePart::OutputText {
        text: completion_content(completion)?.to_owned(),
        annotations: completion_annotations(completion)?,
        logprobs: RequiredNullable::Null,
        extra: Extra::new(),
    })
}

fn completed_message_item(item_id: &str, text: &str, annotations: &[Extra]) -> Extra {
    let objects: Vec<Json> = annotations
        .iter()
        .map(|fields| {
            Json::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        })
        .collect();
    let content_part = Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("output_text".to_owned())),
        ("text".to_owned(), Json::Text(text.to_owned())),
        ("annotations".to_owned(), Json::List(objects)),
    ]));
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("status".to_owned(), Json::Text("completed".to_owned())),
        ("type".to_owned(), Json::Text("message".to_owned())),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("content".to_owned(), Json::List(vec![content_part])),
    ])
}

fn reasoning_item(item_id: &str, content: &str) -> Extra {
    let part = Json::Object(summary_part(content).into_iter().collect());
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("type".to_owned(), Json::Text("reasoning".to_owned())),
        ("summary".to_owned(), Json::List(vec![part])),
    ])
}

fn summary_part(content: &str) -> Extra {
    Extra::from([
        ("type".to_owned(), Json::Text("summary_text".to_owned())),
        ("text".to_owned(), Json::Text(content.to_owned())),
    ])
}

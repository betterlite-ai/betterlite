use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartDoneEvent, ContentPartDonePart, Extra, OutputItemDoneEvent, OutputTextDoneEvent,
    ReasoningSummaryPartDoneEvent, ReasoningSummaryTextDoneEvent, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_tools::state::ChatStreamState;

use super::completion::{completion_annotations, completion_content};
use super::error::StreamItemsError;
use super::items::ensure_cached_item_id;
use super::sources::EventSources;

pub fn create_output_text_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    let text = completion_content(completion)?;
    Ok(KnownResponseEvent::OutputTextDone(OutputTextDoneEvent {
        item_id,
        output_index: state.progress.message_output_index,
        content_index: 0,
        text: text.to_owned(),
        extra: Extra::new(),
    }))
}

pub fn create_output_content_part_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    let text = completion_content(completion)?;
    let annotations = completion_annotations(completion)?;
    Ok(KnownResponseEvent::ContentPartDone(ContentPartDoneEvent {
        item_id,
        output_index: state.progress.message_output_index,
        content_index: 0,
        part: ContentPartDonePart::OutputText {
            text: text.to_owned(),
            annotations,
            logprobs: RequiredNullable::Null,
            extra: Extra::new(),
        },
    }))
}

// LiteLLM's builder ignores its completion argument here and reads self.litellm_model_response instead.
pub fn create_output_item_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    _completion: &Json,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    let Some(stored) = state.progress.litellm_model_response.clone() else {
        return Err(StreamItemsError::MissingModelResponse);
    };
    let text = completion_content(&stored)?;
    let annotations = completion_annotations(&stored)?;
    Ok(KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
        output_index: state.progress.message_output_index,
        sequence_number: 1,
        item: completed_message_item(&item_id, text, annotations),
        extra: Extra::new(),
    }))
}

#[must_use]
pub fn create_reasoning_summary_text_done_event(
    reasoning_item_id: &str,
    reasoning_content: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::ReasoningSummaryTextDone(ReasoningSummaryTextDoneEvent {
        item_id: reasoning_item_id.to_owned(),
        output_index: 0,
        sequence_number,
        summary_index: 0,
        text: reasoning_content.to_owned(),
        extra: Extra::new(),
    })
}

#[must_use]
pub fn create_reasoning_summary_part_done_event(
    reasoning_item_id: &str,
    reasoning_content: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::ReasoningSummaryPartDone(ReasoningSummaryPartDoneEvent {
        item_id: reasoning_item_id.to_owned(),
        output_index: 0,
        sequence_number,
        summary_index: 0,
        part: summary_part(reasoning_content),
        extra: Extra::new(),
    })
}

#[must_use]
pub fn create_reasoning_output_item_done_event(
    reasoning_item_id: &str,
    reasoning_content: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
        output_index: 0,
        sequence_number,
        item: reasoning_item(reasoning_item_id, reasoning_content),
        extra: Extra::new(),
    })
}

fn completed_message_item(item_id: &str, text: &str, annotations: Vec<Extra>) -> Extra {
    let content_part = Extra::from([
        ("type".to_owned(), Json::Text("output_text".to_owned())),
        ("text".to_owned(), Json::Text(text.to_owned())),
        (
            "annotations".to_owned(),
            Json::List(annotations.into_iter().map(Json::Object).collect()),
        ),
    ]);
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("status".to_owned(), Json::Text("completed".to_owned())),
        ("type".to_owned(), Json::Text("message".to_owned())),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        (
            "content".to_owned(),
            Json::List(vec![Json::Object(content_part)]),
        ),
    ])
}

fn reasoning_item(item_id: &str, content: &str) -> Extra {
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("type".to_owned(), Json::Text("reasoning".to_owned())),
        (
            "summary".to_owned(),
            Json::List(vec![Json::Object(summary_part(content))]),
        ),
    ])
}

fn summary_part(content: &str) -> Extra {
    Extra::from([
        ("type".to_owned(), Json::Text("summary_text".to_owned())),
        ("text".to_owned(), Json::Text(content.to_owned())),
    ])
}

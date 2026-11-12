use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    Extra, FunctionArgsDelta, FunctionArgsDone, IndexedItemEvent, OutputItemAddedEvent,
    OutputItemDoneEvent, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use serde_json::Number;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSearchStage {
    InProgress,
    Searching,
    Completed,
}

fn numbered(sequence_number: i64) -> Extra {
    let mut extra = Extra::new();
    extra.insert(
        String::from("sequence_number"),
        Json::Number(Number::from(sequence_number)),
    );
    extra
}

#[must_use]
pub fn output_item_added(
    output_index: i64,
    item: Extra,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::OutputItemAdded(OutputItemAddedEvent {
        output_index,
        item: RequiredNullable::Value(item),
        extra: numbered(sequence_number),
    })
}

#[must_use]
pub fn arguments_delta(
    item_id: &str,
    output_index: i64,
    delta: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::FunctionCallArgumentsDelta(FunctionArgsDelta {
        item_id: item_id.to_owned(),
        output_index,
        delta: delta.to_owned(),
        extra: numbered(sequence_number),
    })
}

#[must_use]
pub fn arguments_done(
    item_id: &str,
    output_index: i64,
    arguments: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::FunctionCallArgumentsDone(FunctionArgsDone {
        item_id: item_id.to_owned(),
        output_index,
        arguments: arguments.to_owned(),
        extra: numbered(sequence_number),
    })
}

#[must_use]
pub const fn output_item_done(
    output_index: i64,
    item: Extra,
    sequence_number: i64,
) -> KnownResponseEvent {
    KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
        output_index,
        sequence_number,
        item,
        extra: Extra::new(),
    })
}

#[must_use]
pub fn web_search_event(
    stage: WebSearchStage,
    output_index: i64,
    item_id: &str,
    sequence_number: i64,
) -> KnownResponseEvent {
    let event = IndexedItemEvent {
        output_index,
        item_id: item_id.to_owned(),
        extra: numbered(sequence_number),
    };
    match stage {
        WebSearchStage::InProgress => KnownResponseEvent::WebSearchCallInProgress(event),
        WebSearchStage::Searching => KnownResponseEvent::WebSearchCallSearching(event),
        WebSearchStage::Completed => KnownResponseEvent::WebSearchCallCompleted(event),
    }
}

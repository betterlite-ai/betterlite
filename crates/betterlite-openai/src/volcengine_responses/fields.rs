use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::response_events::event_decode::{EventClass, KnownClass};

// the four defaults LiteLLM's _default_for_annotation picks, by the kind each annotation declares
#[derive(Clone, Copy)]
enum Kind {
    Integer,
    List,
    UnionWithList,
    Other,
}

// the nested records LiteLLM's _maybe_fill_nested fills when a field carries one
#[derive(Clone, Copy)]
enum Record {
    Plain,
    Response,
    Usage,
    Part,
    ErrorObject,
}

type Field = (&'static str, Kind, Option<Record>);

const RESPONSE_EVENT: &[Field] = &[("response", Kind::Other, Some(Record::Response))];

const OUTPUT_ITEM: &[Field] = &[
    ("output_index", Kind::Integer, None),
    ("item", Kind::Other, Some(Record::Plain)),
];

const CONTENT_PART_ADDED: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("part", Kind::Other, Some(Record::Plain)),
];

const CONTENT_PART_DONE: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("part", Kind::Other, Some(Record::Part)),
];

const CONTENT_DELTA: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("delta", Kind::Other, None),
];

const CONTENT_TEXT_DONE: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("text", Kind::Other, None),
];

const CONTENT_REFUSAL_DONE: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("refusal", Kind::Other, None),
];

const ANNOTATION_ADDED: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("content_index", Kind::Integer, None),
    ("annotation_index", Kind::Integer, None),
    ("annotation", Kind::Other, None),
];

const FUNCTION_DELTA: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("delta", Kind::Other, None),
];

const FUNCTION_DONE: &[Field] = &[
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
    ("arguments", Kind::Other, None),
];

const SEARCH_CALL: &[Field] = &[
    ("output_index", Kind::Integer, None),
    ("item_id", Kind::Other, None),
];

const MCP_CALL: &[Field] = &[
    ("sequence_number", Kind::Integer, None),
    ("output_index", Kind::Integer, None),
    ("item_id", Kind::Other, None),
];

const MCP_ARGS_DELTA: &[Field] = &[
    ("output_index", Kind::Integer, None),
    ("item_id", Kind::Other, None),
    ("delta", Kind::Other, None),
    ("sequence_number", Kind::Integer, None),
];

const MCP_ARGS_DONE: &[Field] = &[
    ("output_index", Kind::Integer, None),
    ("item_id", Kind::Other, None),
    ("arguments", Kind::Other, None),
    ("sequence_number", Kind::Integer, None),
];

const MCP_TERMINAL: &[Field] = &[
    ("sequence_number", Kind::Integer, None),
    ("item_id", Kind::Other, None),
    ("output_index", Kind::Integer, None),
];

const IMAGE_PARTIAL: &[Field] = &[
    ("partial_image_index", Kind::Integer, None),
    ("b64_json", Kind::Other, None),
];

const ERROR_EVENT: &[Field] = &[
    ("sequence_number", Kind::Integer, None),
    ("error", Kind::Other, Some(Record::ErrorObject)),
];

const GENERIC_EVENT: &[Field] = &[("type", Kind::Other, None)];

const RESPONSE_OBJECT: &[Field] = &[
    ("id", Kind::Other, None),
    ("created_at", Kind::Integer, None),
    ("output", Kind::UnionWithList, None),
    ("tools", Kind::UnionWithList, None),
    ("usage", Kind::Other, Some(Record::Usage)),
];

const USAGE_OBJECT: &[Field] = &[
    ("input_tokens", Kind::Integer, None),
    ("output_tokens", Kind::Integer, None),
    ("total_tokens", Kind::Integer, None),
];

const PART_OUTPUT_TEXT: &[Field] = &[
    ("text", Kind::Other, None),
    ("annotations", Kind::List, Some(Record::Plain)),
    ("logprobs", Kind::UnionWithList, None),
];

const PART_REFUSAL: &[Field] = &[("refusal", Kind::Other, None)];

const PART_REASONING: &[Field] = &[("reasoning", Kind::Other, None)];

const ERROR_OBJECT: &[Field] = &[
    ("type", Kind::Other, None),
    ("code", Kind::Other, None),
    ("message", Kind::Other, None),
];

fn default_for(kind: Kind) -> Json {
    match kind {
        Kind::Integer => Json::Number(Number::from(0_u8)),
        Kind::List | Kind::UnionWithList => Json::List(Vec::new()),
        Kind::Other => Json::Null,
    }
}

fn fill(fields: &mut IndexMap<String, Json>, table: &[Field]) {
    for &(name, kind, nested) in table {
        if let Some(value) = fields.get_mut(name) {
            fill_nested(value, nested);
            continue;
        }
        fields.insert(String::from(name), default_for(kind));
    }
}

// the content_part.done part is a union LiteLLM picks by its type literal, else the first member
fn record_table(record: Record, fields: &IndexMap<String, Json>) -> &'static [Field] {
    let given = fields.get("type");
    match record {
        Record::Plain => &[],
        Record::Response => RESPONSE_OBJECT,
        Record::Usage => USAGE_OBJECT,
        Record::ErrorObject => ERROR_OBJECT,
        Record::Part => match given {
            Some(Json::Text(name)) if name == "refusal" => PART_REFUSAL,
            Some(Json::Text(name)) if name == "reasoning_text" => PART_REASONING,
            _ => PART_OUTPUT_TEXT,
        },
    }
}

fn fill_nested(value: &mut Json, nested: Option<Record>) {
    let Some(record) = nested else {
        return;
    };
    match value {
        Json::Object(fields) => {
            let table = record_table(record, fields);
            fill(fields, table);
        }
        Json::List(items) => {
            for item in items {
                if let Json::Object(fields) = item {
                    let table = record_table(record, fields);
                    fill(fields, table);
                }
            }
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

const fn event_table(known: KnownClass) -> &'static [Field] {
    match known {
        KnownClass::ResponseCreated
        | KnownClass::ResponseInProgress
        | KnownClass::ResponseCompleted
        | KnownClass::ResponseFailed
        | KnownClass::ResponseIncomplete => RESPONSE_EVENT,
        KnownClass::OutputItemAdded | KnownClass::OutputItemDone => OUTPUT_ITEM,
        KnownClass::ContentPartAdded => CONTENT_PART_ADDED,
        KnownClass::ContentPartDone => CONTENT_PART_DONE,
        KnownClass::OutputTextDelta | KnownClass::RefusalDelta => CONTENT_DELTA,
        KnownClass::OutputTextDone => CONTENT_TEXT_DONE,
        KnownClass::RefusalDone => CONTENT_REFUSAL_DONE,
        KnownClass::OutputTextAnnotationAdded => ANNOTATION_ADDED,
        KnownClass::FunctionCallArgumentsDelta => FUNCTION_DELTA,
        KnownClass::FunctionCallArgumentsDone => FUNCTION_DONE,
        KnownClass::FileSearchCallInProgress
        | KnownClass::FileSearchCallSearching
        | KnownClass::FileSearchCallCompleted
        | KnownClass::WebSearchCallInProgress
        | KnownClass::WebSearchCallSearching
        | KnownClass::WebSearchCallCompleted => SEARCH_CALL,
        KnownClass::McpListToolsInProgress
        | KnownClass::McpListToolsCompleted
        | KnownClass::McpListToolsFailed
        | KnownClass::McpCallInProgress => MCP_CALL,
        KnownClass::McpCallArgumentsDelta => MCP_ARGS_DELTA,
        KnownClass::McpCallArgumentsDone => MCP_ARGS_DONE,
        KnownClass::McpCallCompleted | KnownClass::McpCallFailed => MCP_TERMINAL,
        KnownClass::ImageGenerationPartialImage => IMAGE_PARTIAL,
        KnownClass::Error => ERROR_EVENT,
    }
}

pub(super) fn fill_event(class: EventClass, fields: &mut IndexMap<String, Json>) {
    match class {
        EventClass::Known(known) => fill(fields, event_table(known)),
        EventClass::Generic => fill(fields, GENERIC_EVENT),
    }
}

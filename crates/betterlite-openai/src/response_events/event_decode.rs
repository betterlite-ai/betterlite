use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

use crate::api::responses_event_records::records::{
    GenericEvent, KnownResponseEvent, ResponsesTerminalEvent,
};
use crate::api::responses_objects::ResponsesApiResponse;
use crate::api::responses_shapes::wire::{
    ShapeDecodeError, WireDecodeLimits, decode_responses_shapes,
};

const DECODE_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 0x4000,
};

macro_rules! event_classes {
    ($($class:ident => $event_type:literal => $variant:ident => $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum KnownClass {
            $($class,)*
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum EventClass {
            Known(KnownClass),
            Generic,
        }

        impl EventClass {
            #[must_use]
            pub fn of(event_type: &str) -> Self {
                match event_type {
                    $($event_type => Self::Known(KnownClass::$class),)*
                    _ => Self::Generic,
                }
            }

            #[must_use]
            pub const fn class_name(self) -> &'static str {
                match self {
                    Self::Known(class) => class.class_name(),
                    Self::Generic => "GenericEvent",
                }
            }
        }

        impl KnownClass {
            #[must_use]
            pub const fn class_name(self) -> &'static str {
                match self {
                    $(Self::$class => $name,)*
                }
            }
        }

        fn decode_known(
            class: KnownClass,
            value: Value,
        ) -> Result<KnownResponseEvent, serde_json::Error> {
            Ok(match class {
                $(KnownClass::$class => KnownResponseEvent::$variant(serde_json::from_value(value)?),)*
            })
        }
    };
}

event_classes! {
    ResponseCreated => "response.created" => ResponseCreated => "ResponseCreatedEvent",
    ResponseInProgress => "response.in_progress" => ResponseInProgress => "ResponseInProgressEvent",
    ResponseCompleted => "response.completed" => ResponseCompleted => "ResponseCompletedEvent",
    ResponseFailed => "response.failed" => ResponseFailed => "ResponseFailedEvent",
    ResponseIncomplete => "response.incomplete" => ResponseIncomplete => "ResponseIncompleteEvent",
    OutputItemAdded => "response.output_item.added" => OutputItemAdded => "OutputItemAddedEvent",
    OutputItemDone => "response.output_item.done" => OutputItemDone => "OutputItemDoneEvent",
    ContentPartAdded => "response.content_part.added" => ContentPartAdded => "ContentPartAddedEvent",
    ContentPartDone => "response.content_part.done" => ContentPartDone => "ContentPartDoneEvent",
    OutputTextDelta => "response.output_text.delta" => OutputTextDelta => "OutputTextDeltaEvent",
    OutputTextAnnotationAdded => "response.output_text.annotation.added" => OutputTextAnnotationAdded => "OutputTextAnnotationAddedEvent",
    OutputTextDone => "response.output_text.done" => OutputTextDone => "OutputTextDoneEvent",
    RefusalDelta => "response.refusal.delta" => RefusalDelta => "RefusalDeltaEvent",
    RefusalDone => "response.refusal.done" => RefusalDone => "RefusalDoneEvent",
    FunctionCallArgumentsDelta => "response.function_call_arguments.delta" => FunctionCallArgumentsDelta => "FunctionCallArgumentsDeltaEvent",
    FunctionCallArgumentsDone => "response.function_call_arguments.done" => FunctionCallArgumentsDone => "FunctionCallArgumentsDoneEvent",
    FileSearchCallInProgress => "response.file_search_call.in_progress" => FileSearchCallInProgress => "FileSearchCallInProgressEvent",
    FileSearchCallSearching => "response.file_search_call.searching" => FileSearchCallSearching => "FileSearchCallSearchingEvent",
    FileSearchCallCompleted => "response.file_search_call.completed" => FileSearchCallCompleted => "FileSearchCallCompletedEvent",
    WebSearchCallInProgress => "response.web_search_call.in_progress" => WebSearchCallInProgress => "WebSearchCallInProgressEvent",
    WebSearchCallSearching => "response.web_search_call.searching" => WebSearchCallSearching => "WebSearchCallSearchingEvent",
    WebSearchCallCompleted => "response.web_search_call.completed" => WebSearchCallCompleted => "WebSearchCallCompletedEvent",
    McpListToolsInProgress => "response.mcp_list_tools.in_progress" => McpListToolsInProgress => "MCPListToolsInProgressEvent",
    McpListToolsCompleted => "response.mcp_list_tools.completed" => McpListToolsCompleted => "MCPListToolsCompletedEvent",
    McpListToolsFailed => "response.mcp_list_tools.failed" => McpListToolsFailed => "MCPListToolsFailedEvent",
    McpCallInProgress => "response.mcp_call.in_progress" => McpCallInProgress => "MCPCallInProgressEvent",
    McpCallArgumentsDelta => "response.mcp_call_arguments.delta" => McpCallArgumentsDelta => "MCPCallArgumentsDeltaEvent",
    McpCallArgumentsDone => "response.mcp_call_arguments.done" => McpCallArgumentsDone => "MCPCallArgumentsDoneEvent",
    McpCallCompleted => "response.mcp_call.completed" => McpCallCompleted => "MCPCallCompletedEvent",
    McpCallFailed => "response.mcp_call.failed" => McpCallFailed => "MCPCallFailedEvent",
    ImageGenerationPartialImage => "image_generation.partial_image" => ImageGenerationPartialImage => "ImageGenerationPartialImageEvent",
    Error => "error" => Error => "ErrorEvent",
}

pub enum DecodedStreamResponse {
    Known {
        class: EventClass,
        record: Box<KnownResponseEvent>,
    },
    Generic(GenericEvent),
    Lax {
        class: EventClass,
        fields: IndexMap<String, Json>,
    },
}

impl DecodedStreamResponse {
    #[must_use]
    pub const fn class(&self) -> EventClass {
        match self {
            Self::Known { class, .. } | Self::Lax { class, .. } => *class,
            Self::Generic(_event) => EventClass::Generic,
        }
    }
}

// LiteLLM's get_event_model_class plus the error.code coalescing and lax construction of transform_streaming_response
#[must_use]
pub fn transform_streaming_response(chunk: &IndexMap<String, Json>) -> DecodedStreamResponse {
    let event_type = match chunk.get("type") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let fields = coalesce_error_code(chunk);
    let Some(event_name) = event_type else {
        return DecodedStreamResponse::Lax {
            class: EventClass::Generic,
            fields,
        };
    };
    let class = EventClass::of(&event_name);
    match class {
        EventClass::Generic => {
            let extra = fields
                .into_iter()
                .filter(|(name, _value)| name != "type")
                .collect();
            DecodedStreamResponse::Generic(GenericEvent {
                r#type: event_name,
                extra,
            })
        }
        EventClass::Known(known) => known_or_lax(class, known, fields),
    }
}

fn known_or_lax(
    class: EventClass,
    known: KnownClass,
    fields: IndexMap<String, Json>,
) -> DecodedStreamResponse {
    let Ok(value) = serde_json::to_value(Json::Object(fields.clone())) else {
        return DecodedStreamResponse::Lax { class, fields };
    };
    match decode_known(known, value) {
        Ok(record) => DecodedStreamResponse::Known {
            class,
            record: Box::new(record),
        },
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the Responses event did not validate; keeping its fields as sent"
            );
            DecodedStreamResponse::Lax { class, fields }
        }
    }
}

fn coalesce_error_code(chunk: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let Some(Json::Object(error)) = chunk.get("error") else {
        return chunk.clone();
    };
    if error
        .get("code")
        .is_some_and(|code| !matches!(code, Json::Null))
    {
        return chunk.clone();
    }
    let mut fixed = chunk.clone();
    let mut patched = error.clone();
    patched.insert(
        String::from("code"),
        Json::Text(String::from("unknown_error")),
    );
    fixed.insert(String::from("error"), Json::Object(patched));
    fixed
}

#[derive(Debug, Error)]
pub enum ResponseEventDecodeError {
    #[error("the Responses event is not valid JSON: {0}")]
    Shape(#[from] ShapeDecodeError),
    #[error("a Responses event must be a JSON object")]
    NotAnObject,
}

// The outside-byte boundary: B005 hands over one event's data bytes, this decodes the record it carries
pub fn decode_response_event(
    bytes: &[u8],
) -> Result<DecodedStreamResponse, ResponseEventDecodeError> {
    let decoded: Json = decode_responses_shapes(bytes, DECODE_LIMITS)?;
    match decoded {
        Json::Object(fields) => Ok(transform_streaming_response(&fields)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ResponseEventDecodeError::NotAnObject)
        }
    }
}

// LiteLLM's parse_terminal_event_from_stream_chunks: reverse order, only an exact "data: " prefix stripped
#[must_use = "a missing terminal event is the answer for a stream that never completed"]
pub fn parse_terminal_event(chunks: &[String]) -> Option<ResponsesTerminalEvent> {
    for chunk in chunks.iter().rev() {
        let payload = chunk.strip_prefix("data: ").unwrap_or(chunk);
        let Ok(event) = serde_json::from_str::<ResponsesTerminalEvent>(payload) else {
            continue;
        };
        return Some(event);
    }
    None
}

#[must_use = "a missing terminal reply is the answer for a stream that never completed"]
pub fn parse_terminal_response(chunks: &[String]) -> Option<ResponsesApiResponse> {
    parse_terminal_event(chunks).map(|event| match event {
        ResponsesTerminalEvent::Completed(response)
        | ResponsesTerminalEvent::Incomplete(response)
        | ResponsesTerminalEvent::Failed(response) => response.response,
    })
}

use betterlite_openai::api::responses_event_records::records::ResponsesTerminalEvent;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, RelayCallType, RelayShape,
};
use betterlite_openai::response_events::event_decode::parse_terminal_event;

pub const RESPONSES_SUFFIX: &str = "/responses";

// B668's projection hook, called with the decoded reply by the composing route's logging state
pub trait ChatReplyProjection {
    fn project_chat_reply(
        &self,
        model: &str,
        reply: &ResponseReply<'_>,
        input: LogInput,
    ) -> Option<Object>;
}

// B3636 owns the assembly behind this seam, and the route owns the token counter it needs
pub trait ChatStreamAssembly {
    fn complete_streaming_response(
        &self,
        chunks: &[String],
        model: &str,
        messages: Option<&[Object]>,
    ) -> Option<Object>;
}

#[derive(Debug, Default)]
pub struct NoChatReplyProjection;

impl ChatReplyProjection for NoChatReplyProjection {
    fn project_chat_reply(
        &self,
        _model: &str,
        _reply: &ResponseReply<'_>,
        _input: LogInput,
    ) -> Option<Object> {
        None
    }
}

#[derive(Debug, Default)]
pub struct NoChatStreamAssembly;

impl ChatStreamAssembly for NoChatStreamAssembly {
    fn complete_streaming_response(
        &self,
        _chunks: &[String],
        _model: &str,
        _messages: Option<&[Object]>,
    ) -> Option<Object> {
        None
    }
}

#[derive(Debug, Clone)]
pub enum AzureRelayedLog {
    Chat(Object),
    Relay(Object),
    Terminal(Box<ResponsesTerminalEvent>),
}

// EmbeddingResponse.model_validate: object is the literal "list" and data is a list
fn embedding_response(body: &Object) -> Option<Object> {
    let object_is_list =
        matches!(body.get("object"), Some(value) if value.as_str() == Some("list"));
    let data_is_list = matches!(body.get("data"), Some(value) if value.is_array());
    if object_is_list && data_is_list {
        return Some(body.clone());
    }
    None
}

// ResponsesAPIResponse.model_validate requires id, created_at and output
fn responses_response(body: &Object) -> Option<Object> {
    let id_is_text = matches!(body.get("id"), Some(value) if value.is_string());
    let created_is_integer = matches!(body.get("created_at"), Some(value) if value.is_i64());
    let output_is_list = matches!(body.get("output"), Some(value) if value.is_array());
    if id_is_text && created_is_integer && output_is_list {
        return Some(body.clone());
    }
    None
}

// ImagesResponse.model_validate requires created and data
fn image_response(body: &Object) -> Option<Object> {
    let created_is_integer = matches!(body.get("created"), Some(value) if value.is_i64());
    let data_is_list = matches!(body.get("data"), Some(value) if value.is_array());
    if created_is_integer && data_is_list {
        return Some(body.clone());
    }
    None
}

pub const OPENAI_RELAY_SHAPES: [RelayShape<'static, Object>; 3] = [
    RelayShape {
        path_suffix: "/embeddings",
        call_type: RelayCallType::Aembedding,
        parse: embedding_response,
    },
    RelayShape {
        path_suffix: RESPONSES_SUFFIX,
        call_type: RelayCallType::Aresponses,
        parse: responses_response,
    },
    RelayShape {
        path_suffix: "/images/generations",
        call_type: RelayCallType::AimageGeneration,
        parse: image_response,
    },
];

// RelayedCallDetails validation: request_data must be an object whose messages are all objects
#[must_use]
pub fn relayed_messages(model_call_details: &Object) -> Option<Vec<Object>> {
    let request_data = model_call_details.get("request_data")?.as_object()?;
    request_data
        .get("messages")?
        .as_array()?
        .iter()
        .map(|message| message.as_object().cloned())
        .collect()
}

pub fn logged_responses_stream(
    all_chunks: &[String],
    logging_obj: &mut dyn PassthroughLogging,
) -> Option<ResponsesTerminalEvent> {
    let terminal_event = parse_terminal_event(all_chunks)?;
    logging_obj.set_call_type(RelayCallType::Aresponses);
    Some(terminal_event)
}

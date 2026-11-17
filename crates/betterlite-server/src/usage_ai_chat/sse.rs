use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Running,
    Complete,
    Error,
}

impl ToolCallStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Complete => "complete",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    Status {
        message: String,
    },
    ToolCall {
        tool_name: String,
        tool_label: String,
        arguments: IndexMap<String, String>,
        status: ToolCallStatus,
    },
    Chunk {
        content: String,
    },
    Done,
    Error {
        message: String,
    },
}

#[must_use]
pub fn sse_frame(event: &SseEvent) -> String {
    let object = match event {
        SseEvent::Status { message } => IndexMap::from([
            ("type".to_owned(), Json::Text("status".to_owned())),
            ("message".to_owned(), Json::Text(message.clone())),
        ]),
        SseEvent::Chunk { content } => IndexMap::from([
            ("type".to_owned(), Json::Text("chunk".to_owned())),
            ("content".to_owned(), Json::Text(content.clone())),
        ]),
        SseEvent::Done => IndexMap::from([("type".to_owned(), Json::Text("done".to_owned()))]),
        SseEvent::Error { message } => IndexMap::from([
            ("type".to_owned(), Json::Text("error".to_owned())),
            ("message".to_owned(), Json::Text(message.clone())),
        ]),
        SseEvent::ToolCall {
            tool_name,
            tool_label,
            arguments,
            status,
        } => IndexMap::from([
            ("type".to_owned(), Json::Text("tool_call".to_owned())),
            ("tool_name".to_owned(), Json::Text(tool_name.clone())),
            ("tool_label".to_owned(), Json::Text(tool_label.clone())),
            (
                "arguments".to_owned(),
                Json::Object(
                    arguments
                        .iter()
                        .map(|(key, value)| (key.clone(), Json::Text(value.clone())))
                        .collect(),
                ),
            ),
            ("status".to_owned(), Json::Text(status.as_str().to_owned())),
        ]),
    };
    format!("data: {}\n\n", dumps_object(&object))
}

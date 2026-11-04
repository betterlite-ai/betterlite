use super::wire::ResponsesAPIStreamEvents as Event;

const RESPONSE: &[&str] = &["response"];
const PART: &[&str] = &["item_id", "output_index", "part"];
const DELTA: &[&str] = &["item_id", "output_index", "delta"];
const SEQUENCE_TEXT: &[&str] = &["item_id", "output_index", "sequence_number", "text"];
const SEQUENCE_PART: &[&str] = &["item_id", "output_index", "sequence_number", "part"];
const ITEM: &[&str] = &["output_index", "item"];
const CONTENT_PART: &[&str] = &["item_id", "output_index", "content_index", "part"];
const INDEXED_DELTA: &[&str] = &["item_id", "output_index", "content_index", "delta"];
const ANNOTATION: &[&str] = &[
    "item_id",
    "output_index",
    "content_index",
    "annotation_index",
    "annotation",
];
const TEXT: &[&str] = &["item_id", "output_index", "content_index", "text"];
const REFUSAL: &[&str] = &["item_id", "output_index", "content_index", "refusal"];
const ARGUMENTS: &[&str] = &["item_id", "output_index", "arguments"];
const SEARCH: &[&str] = &["output_index", "item_id"];
const MCP: &[&str] = &["sequence_number", "output_index", "item_id"];
const MCP_DELTA: &[&str] = &["output_index", "item_id", "delta", "sequence_number"];
const MCP_ARGUMENTS: &[&str] = &["output_index", "item_id", "arguments", "sequence_number"];
const MCP_TERMINAL: &[&str] = &["sequence_number", "item_id", "output_index"];
const IMAGE: &[&str] = &["partial_image_index", "b64_json"];
const ERROR: &[&str] = &["sequence_number", "error"];
const TYPE: &[&str] = &["type"];

const EVENT_FIELDS: [(Event, &[&str]); 41] = [
    (Event::ResponseCreated, RESPONSE),
    (Event::ResponseInProgress, RESPONSE),
    (Event::ResponseCompleted, RESPONSE),
    (Event::ResponseFailed, RESPONSE),
    (Event::ResponseIncomplete, RESPONSE),
    (Event::ResponsePartAdded, PART),
    (Event::ReasoningSummaryTextDelta, DELTA),
    (Event::ReasoningSummaryTextDone, SEQUENCE_TEXT),
    (Event::ReasoningSummaryPartDone, SEQUENCE_PART),
    (Event::OutputItemAdded, ITEM),
    (Event::OutputItemDone, ITEM),
    (Event::ContentPartAdded, CONTENT_PART),
    (Event::ContentPartDone, CONTENT_PART),
    (Event::OutputTextDelta, INDEXED_DELTA),
    (Event::OutputTextAnnotationAdded, ANNOTATION),
    (Event::OutputTextDone, TEXT),
    (Event::RefusalDelta, INDEXED_DELTA),
    (Event::RefusalDone, REFUSAL),
    (Event::FunctionCallArgumentsDelta, DELTA),
    (Event::FunctionCallArgumentsDone, ARGUMENTS),
    (Event::CustomToolCallInputDelta, TYPE),
    (Event::CustomToolCallInputDone, TYPE),
    (Event::FileSearchCallInProgress, SEARCH),
    (Event::FileSearchCallSearching, SEARCH),
    (Event::FileSearchCallCompleted, SEARCH),
    (Event::WebSearchCallInProgress, SEARCH),
    (Event::WebSearchCallSearching, SEARCH),
    (Event::WebSearchCallCompleted, SEARCH),
    (Event::McpListToolsInProgress, MCP),
    (Event::McpListToolsCompleted, MCP),
    (Event::McpListToolsFailed, MCP),
    (Event::McpCallInProgress, MCP),
    (Event::McpCallArgumentsDelta, MCP_DELTA),
    (Event::McpCallArgumentsDone, MCP_ARGUMENTS),
    (Event::McpCallCompleted, MCP_TERMINAL),
    (Event::McpCallFailed, MCP_TERMINAL),
    (Event::ImageGenerationPartialImage, IMAGE),
    (Event::ShellCallInProgress, TYPE),
    (Event::ShellCallCompleted, TYPE),
    (Event::ShellCallOutput, TYPE),
    (Event::Error, ERROR),
];

pub(super) fn required_fields(event: Event) -> &'static [&'static str] {
    if let Some((_, fields)) = EVENT_FIELDS.iter().find(|(kind, _)| *kind == event) {
        fields
    } else {
        &[]
    }
}

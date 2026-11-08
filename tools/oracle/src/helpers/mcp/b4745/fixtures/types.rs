use std::error::Error;

use betterlite_mcp::client::tools::result::{
    CallToolResult, ContentBlock, TextContent, ToolCallResult,
};
use betterlite_mcp::tools::pagination::Page;
use betterlite_mcp::tools::schemas::McpTool;
use betterlite_prompts::arguments::json::Object;
use tokio_util::sync::CancellationToken;

use super::super::input::{CallScenario, ErrorKind, Frame, ListScenario};

// The SDK's exception classes are distinct types, so a plain failure and a broken connection are separate source types
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct SimulatedError(pub(crate) &'static str);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the connection is broken")]
pub(crate) struct BrokenResourceError;

pub(crate) trait Simulated: Error + Send + Sync + Clone + 'static {
    fn simulated(message: &'static str) -> Self;
}

impl Simulated for SimulatedError {
    fn simulated(message: &'static str) -> Self {
        Self(message)
    }
}

impl Simulated for BrokenResourceError {
    fn simulated(_message: &'static str) -> Self {
        Self
    }
}

const fn listing_message(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Plain => "upstream failure: listing failed",
        ErrorKind::Broken => "the connection is broken",
    }
}

const fn calling_message(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Plain => "upstream failure: tool crashed",
        ErrorKind::Broken => "the connection is broken",
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Script<Source> {
    pub(crate) frames: Vec<Frame>,
    pub(crate) list_error: Option<Source>,
    pub(crate) call_error: Option<Source>,
    pub(crate) call: Option<CallScenario>,
    pub(crate) cancel: bool,
    pub(crate) cancelled: CancellationToken,
}

#[derive(Clone, Copy)]
pub(crate) enum CallMode<'scenario> {
    Listing(&'scenario ListScenario),
    Calling(&'scenario CallScenario),
}

impl<Source> Script<Source>
where
    Source: Simulated,
{
    pub(crate) fn new(mode: CallMode<'_>) -> Self {
        match mode {
            CallMode::Listing(scenario) => Self {
                frames: scenario.pages.clone(),
                list_error: scenario
                    .error
                    .map(|kind| Source::simulated(listing_message(kind))),
                call_error: None,
                call: None,
                cancel: scenario.cancel,
                cancelled: CancellationToken::new(),
            },
            CallMode::Calling(scenario) => Self {
                frames: Vec::new(),
                list_error: None,
                call_error: scenario
                    .error
                    .map(|kind| Source::simulated(calling_message(kind))),
                call: Some(scenario.clone()),
                cancel: scenario.cancel,
                cancelled: CancellationToken::new(),
            },
        }
    }
}

pub(crate) fn text_result(text: &str) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: text.to_owned(),
            extra: Object::new(),
        })],
        is_error: false,
        structured_content: None,
        meta: None,
    }
}

pub(crate) fn complete(text: &str) -> ToolCallResult {
    ToolCallResult::Complete(text_result(text))
}

pub(crate) fn page_of(frame: &Frame) -> Page {
    Page {
        tools: frame
            .tools
            .iter()
            .map(|tool| McpTool {
                name: tool.name.clone(),
                description: None,
                input_schema: Some(Object::new()),
                extra: Object::new(),
            })
            .collect(),
        next_cursor: frame.next_cursor.clone(),
    }
}

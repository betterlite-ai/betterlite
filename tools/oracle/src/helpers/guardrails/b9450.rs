mod mcp;
mod projection;
mod stream;
mod support;

use betterlite_models::decisions::Json as ContentJson;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamArgs {
    settings: Value,
    request_data: Value,
    chunks: Vec<Chunk>,
    should_run: bool,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: UpstreamAnswer,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpArgs {
    settings: Value,
    kwargs: IndexMap<String, ContentJson>,
    tool_response: Vec<ContentJson>,
    answer: UpstreamAnswer,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Stream(StreamArgs),
    Mcp(McpArgs),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Stream(args) => stream::run(args),
            Input::Mcp(args) => mcp::run(args),
        }
    }
}

use thiserror::Error;

use crate::api::skills::hook::tool_calls::ToolCallError;

#[derive(Debug, Error)]
pub enum SkillsLoopError {
    #[error("the tool calls of the model response could not be read")]
    ToolCalls(#[from] ToolCallError),
}

use thiserror::Error;

use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, Error)]
pub enum GeminiToolsError {
    #[error("tool_choice must be \"auto\", \"required\", \"none\" or a function object")]
    UnsupportedToolChoice,
    #[error("tool_choice.{0} is not valid")]
    ToolChoice(&'static str),
    #[error("a function declaration's {0} is not valid")]
    Declaration(&'static str),
    #[error("the {tool} tool configuration must be an object")]
    ToolValue { tool: &'static str },
    #[error("shaping the function parameters failed")]
    Shape(#[source] VertexSchemaError),
}

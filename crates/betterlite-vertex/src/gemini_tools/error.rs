use thiserror::Error;

use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, Error)]
pub enum GeminiToolsError {
    #[error("tool_choice must be \"auto\", \"required\", \"none\" or a function object")]
    UnsupportedToolChoice,
    #[error("tool_choice.{field} is not valid")]
    ToolChoiceField { field: &'static str },
    #[error("a function declaration's {field} is not valid")]
    FunctionField { field: &'static str },
    #[error("the {tool} tool configuration must be an object")]
    ToolValue { tool: &'static str },
    #[error("shaping the function parameters failed")]
    Shape {
        #[source]
        source: VertexSchemaError,
    },
}

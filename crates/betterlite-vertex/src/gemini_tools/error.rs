use thiserror::Error;

use crate::vertex_schema::error::VertexSchemaError;

#[derive(Debug, Error)]
pub enum GeminiToolsError {
    #[error("tool_choice must be \"auto\", \"required\", \"none\" or a function object")]
    UnsupportedToolChoice,
    #[error("tool_choice.function must be an object")]
    ToolChoiceFunction,
    #[error("tool_choice.function.name must be a string")]
    ToolChoiceName,
    #[error("a tool's function must be an object")]
    Function,
    #[error("a function declaration needs a string name")]
    FunctionName,
    #[error("a function declaration's description must be a string")]
    Description,
    #[error("function parameters must be a schema object or an empty string")]
    Parameters,
    #[error("the {tool} tool configuration must be an object")]
    ToolValue { tool: &'static str },
    #[error("shaping the function parameters failed")]
    Shape {
        #[source]
        source: VertexSchemaError,
    },
}

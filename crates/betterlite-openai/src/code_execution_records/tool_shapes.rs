use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::{CODE_EXECUTION_TOOL_NAME, CallDialect};

const FUNCTION_DESCRIPTION: &str = "Execute python code in a sandbox and return stdout.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodeExecutionParameters {
    #[serde(rename = "type")]
    pub kind: String,
    pub properties: BTreeMap<String, BTreeMap<String, String>>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResponsesFunctionTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub description: String,
    pub parameters: CodeExecutionParameters,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: CodeExecutionParameters,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatFunctionTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionDefinition,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CodeExecutionTool {
    Responses(ResponsesFunctionTool),
    ChatCompletion(ChatFunctionTool),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResponsesFunctionToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatFunctionToolChoiceFunction {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatFunctionToolChoice {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionToolChoiceFunction,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CodeExecutionToolChoice {
    Responses(ResponsesFunctionToolChoice),
    ChatCompletion(ChatFunctionToolChoice),
}

#[must_use]
pub fn function_parameters() -> CodeExecutionParameters {
    CodeExecutionParameters {
        kind: "object".to_owned(),
        properties: BTreeMap::from([(
            "code".to_owned(),
            BTreeMap::from([("type".to_owned(), "string".to_owned())]),
        )]),
        required: vec!["code".to_owned()],
    }
}

#[must_use]
pub fn function_tool(dialect: CallDialect) -> CodeExecutionTool {
    match dialect {
        CallDialect::ChatCompletion => CodeExecutionTool::ChatCompletion(ChatFunctionTool {
            kind: "function".to_owned(),
            function: ChatFunctionDefinition {
                name: CODE_EXECUTION_TOOL_NAME.to_owned(),
                description: FUNCTION_DESCRIPTION.to_owned(),
                parameters: function_parameters(),
            },
        }),
        CallDialect::Responses => CodeExecutionTool::Responses(ResponsesFunctionTool {
            kind: "function".to_owned(),
            name: CODE_EXECUTION_TOOL_NAME.to_owned(),
            description: FUNCTION_DESCRIPTION.to_owned(),
            parameters: function_parameters(),
        }),
    }
}

#[must_use]
pub fn function_tool_choice(dialect: CallDialect) -> CodeExecutionToolChoice {
    match function_tool(dialect) {
        CodeExecutionTool::ChatCompletion(tool) => {
            CodeExecutionToolChoice::ChatCompletion(ChatFunctionToolChoice {
                kind: tool.kind,
                function: ChatFunctionToolChoiceFunction {
                    name: tool.function.name,
                },
            })
        }
        CodeExecutionTool::Responses(tool) => {
            CodeExecutionToolChoice::Responses(ResponsesFunctionToolChoice {
                kind: tool.kind,
                name: tool.name,
            })
        }
    }
}

pub fn tool_choice_detection(tool_choice: &Value) -> bool {
    let Some(choice) = tool_choice.as_object() else {
        return false;
    };
    let targets_tool_name = |name: &Value| name.as_str() == Some(CODE_EXECUTION_TOOL_NAME);
    choice.get("type").and_then(Value::as_str) == Some("code_interpreter")
        || choice.get("name").and_then(Value::as_str) == Some("code_interpreter")
        || choice.get("name").is_some_and(targets_tool_name)
        || choice
            .get("function")
            .and_then(Value::as_object)
            .and_then(|function| function.get("name"))
            .is_some_and(targets_tool_name)
}

include!("tool_shapes/tests/cases.rs");

use std::error::Error as StdError;

use serde_json::Value;
use thiserror::Error;

use super::lifecycle::{LifecycleError, SandboxConfig, SandboxProvider, SandboxRegistry};
use super::resolution::{SandboxToolParams, SandboxToolRegistry};

pub const INVALID_ARGUMENTS_MESSAGE: &str = "[invalid tool arguments: could not parse code]";
const EXECUTION_ERROR_PREFIX: &str = "[execution error] ";

#[derive(Debug, Error)]
pub enum ToolExecutionError<E>
where
    E: StdError + Send + Sync + 'static,
{
    #[error("code-execution tool-call arguments did not parse into a JSON object")]
    ArgumentsNotAnObject,
    #[error(transparent)]
    Lifecycle(#[from] LifecycleError<E>),
}

enum ExtractCodeError {
    Invalid,
    NotAnObject,
}

fn extract_code(arguments: &str) -> Result<String, ExtractCodeError> {
    if arguments.is_empty() {
        return Ok(String::new());
    }
    let parsed: Value =
        serde_json::from_str(arguments).map_err(|_error| ExtractCodeError::Invalid)?;
    let object = parsed.as_object().ok_or(ExtractCodeError::NotAnObject)?;
    Ok(object
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

fn render_error_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            value.to_string()
        }
    }
}

#[must_use]
pub fn error_message(result: &Value) -> Option<String> {
    let error = result.as_object()?.get("error")?.as_object()?;
    let value = error.get("value").filter(|entry| !entry.is_null());
    let fallback = error.get("name").filter(|entry| !entry.is_null());
    let message = value.or(fallback)?;
    Some(format!(
        "{EXECUTION_ERROR_PREFIX}{}",
        render_error_text(message)
    ))
}

#[must_use]
pub fn stdout(result: &Value) -> String {
    result
        .as_object()
        .and_then(|object| object.get("stdout"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub async fn run_tool_call<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    params: Option<&SandboxToolParams>,
    container: &Value,
    arguments: &str,
) -> Result<String, ToolExecutionError<C::Error>>
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    let code = match extract_code(arguments) {
        Ok(code) => code,
        Err(ExtractCodeError::Invalid) => return Ok(INVALID_ARGUMENTS_MESSAGE.to_owned()),
        Err(ExtractCodeError::NotAnObject) => return Err(ToolExecutionError::ArgumentsNotAnObject),
    };
    let result = provider.run_code(params, container, &code).await?;
    Ok(error_message(&result).unwrap_or_else(|| stdout(&result)))
}

mod tests;

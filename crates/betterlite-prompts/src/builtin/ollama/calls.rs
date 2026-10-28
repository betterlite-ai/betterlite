use super::super::{Field, PromptError, ToolCall};
use super::json;
use serde::Serialize;

#[derive(Serialize)]
struct Call<'input> {
    id: &'input str,
    #[serde(rename = "type")]
    kind: &'static str,
    function: Function<'input>,
}
#[derive(Serialize)]
struct Function<'input> {
    name: &'input str,
    arguments: Field,
}
pub(super) fn render(calls: &[ToolCall]) -> Result<String, PromptError> {
    let converted = calls
        .iter()
        .map(|call| {
            Ok(Call {
                id: &call.id,
                kind: "function",
                function: Function {
                    name: &call.function.name,
                    arguments: serde_json::from_str(&call.function.arguments)?,
                },
            })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()
        .map_err(|source| PromptError::ToolArguments { source })?;
    json::render(&converted, false)
}

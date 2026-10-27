use super::super::{Field, PromptError, ToolCall, bounds};
use serde::Serialize;
use std::fmt::Write as _;

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
    let pretty = serde_json::to_string_pretty(&converted)
        .map_err(|source| PromptError::ToolArguments { source })?;
    let mut output = String::new();
    for character in pretty.chars() {
        if character < '\u{7f}' {
            output.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0; 2]) {
                write!(output, "\\u{unit:04x}")?;
            }
        }
    }
    bounds::finish(output)
}

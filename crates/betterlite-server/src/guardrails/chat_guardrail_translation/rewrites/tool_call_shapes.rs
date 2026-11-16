use super::response::ChatResponse;

mod tests;

pub type ToolCallShape = (Option<String>, String);

// Every function tool call of every choice, flattened in order as (name, arguments).
#[must_use]
pub fn function_tool_call_shapes(response: &ChatResponse) -> Vec<ToolCallShape> {
    response
        .choices
        .iter()
        .flat_map(|choice| choice.message.tool_calls.iter().flatten())
        .filter_map(|call| call.function.as_ref())
        .map(|function| {
            (
                function.name.clone(),
                function.arguments.clone().unwrap_or_default(),
            )
        })
        .collect()
}

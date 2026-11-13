use betterlite_prompts::arguments::json::{Json, Object};

fn non_blank_arguments(function: &Json) -> Option<String> {
    let arguments = function.as_object()?.get("arguments")?.as_str()?;
    Some(arguments)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
}

// Takes the message's own fields so a request message and a model-generated response message read the same way.
#[must_use]
pub fn tool_call_arguments(message: &Object) -> Vec<String> {
    let mut arguments = Vec::new();
    if let Some(Json::List(tool_calls)) = message.get("tool_calls") {
        for tool_call in tool_calls {
            let function = tool_call.as_object().and_then(|call| call.get("function"));
            arguments.extend(function.and_then(non_blank_arguments));
        }
    }
    arguments.extend(message.get("function_call").and_then(non_blank_arguments));
    arguments
}

mod tests;

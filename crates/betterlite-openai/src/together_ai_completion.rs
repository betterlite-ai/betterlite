use serde_json::{Map, Value};
use thiserror::Error;

use crate::text_completion_policy::prompt;

mod tests;

#[derive(Debug, Error)]
pub enum TogetherCompletionError {
    #[error(transparent)]
    Prompt(#[from] prompt::PromptError),
    #[error("TogetherAI does not support integers as input")]
    Integers,
    #[error("TogetherAI does not support multiple prompts.")]
    MultiplePrompts,
}

// LiteLLM runs its shared prompt projection first and then refuses the two shapes Together cannot take
pub fn project_prompt(initial: Value) -> Result<Value, TogetherCompletionError> {
    if prompt::is_tokens_or_list_of_tokens(&initial) {
        return Err(TogetherCompletionError::Integers);
    }
    if let Value::Array(items) = &initial {
        return match items.as_slice() {
            [Value::String(text)] => Ok(Value::String(text.clone())),
            _ => Err(TogetherCompletionError::MultiplePrompts),
        };
    }
    Ok(initial)
}

pub fn transform_completion_request(
    model: &str,
    messages: &[Value],
    optional_params: Map<String, Value>,
) -> Result<Map<String, Value>, TogetherCompletionError> {
    let prompt = project_prompt(prompt::transform_prompt(messages)?)?;
    let mut request = Map::new();
    request.insert(String::from("model"), Value::String(model.to_owned()));
    request.insert(String::from("prompt"), prompt);
    request.extend(optional_params);
    Ok(request)
}

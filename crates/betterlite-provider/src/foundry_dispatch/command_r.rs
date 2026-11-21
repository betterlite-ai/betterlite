use betterlite_prompts::azure::error::AzureError;
use betterlite_prompts::azure::stringify::stringify_tool_content;
use betterlite_prompts::message_content::Message;

// LiteLLM gates the helper on the case-sensitive substring, so Command-R keeps its messages untouched
#[must_use]
pub fn triggers(model: &str) -> bool {
    model.contains("command-r")
}

#[derive(Debug, thiserror::Error)]
pub enum CommandRError {
    #[error(transparent)]
    Stringify(#[from] AzureError),
}

pub fn transform(messages: &[Message], model: &str) -> Result<Vec<Message>, CommandRError> {
    if !triggers(model) {
        return Ok(messages.to_vec());
    }
    Ok(stringify_tool_content(messages)?)
}

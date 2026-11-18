use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

// AmazonAnthropicClaudeMessagesConfig.__init__ calls BaseAnthropicMessagesConfig.__init__ first; that class defines no __init__, so object's initializer rejects every keyword before AmazonInvokeConfig.__init__ runs, and the config has no state of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagesConfig;

// CPython's message when a class without its own __init__ receives a keyword argument.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConstructorError {
    #[error("object.__init__() takes exactly one argument (the instance to initialize)")]
    Arguments,
}

impl TryFrom<&Object> for MessagesConfig {
    type Error = ConstructorError;

    fn try_from(kwargs: &Object) -> Result<Self, Self::Error> {
        if kwargs.is_empty() {
            return Ok(Self);
        }
        Err(ConstructorError::Arguments)
    }
}

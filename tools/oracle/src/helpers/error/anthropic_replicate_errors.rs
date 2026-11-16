use betterlite_error::anthropic_replicate_errors::anthropic::{self, AnthropicExceptionInput};
use betterlite_error::anthropic_replicate_errors::replicate::{self, ReplicateExceptionInput};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Anthropic(Box<AnthropicExceptionInput>),
    Replicate(Box<ReplicateExceptionInput>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match input {
            Input::Anthropic(anthropic_input) => {
                anthropic::map_anthropic_exception(anthropic_input)
            }
            Input::Replicate(replicate_input) => {
                Some(replicate::map_replicate_exception(replicate_input))
            }
        };
        Ok(serde_json::json!({"result": result}))
    }
}

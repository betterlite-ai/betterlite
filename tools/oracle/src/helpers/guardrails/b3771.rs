use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod generic;
mod grayswan;
mod support;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
        if input.get("generic_call").is_some() {
            generic::call(input)
        } else if input.get("grayswan_call").is_some() {
            grayswan::call(input)
        } else if input.get("grayswan_payload").is_some() {
            grayswan::payload(input)
        } else if input.get("metadata").is_some() || input.get("litellm_metadata").is_some() {
            generic::metadata(input)
        } else if input.get("score").is_some() || input.get("action").is_some() {
            grayswan::verdict(input)
        } else {
            Self::not_built("b3771")
        }
    }
}

mod tests;

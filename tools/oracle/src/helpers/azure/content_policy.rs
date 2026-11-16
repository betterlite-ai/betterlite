use betterlite_azure::azure_content_policy::{ContentPolicyInput, content_policy_error};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = ContentPolicyInput;

    fn run(input: &Self::Input) -> HelperResult {
        serde_json::to_value(content_policy_error(input)).map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

mod tests;

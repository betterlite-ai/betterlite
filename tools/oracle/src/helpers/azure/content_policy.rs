use betterlite_azure::azure_content_policy::{ContentPolicyInput, content_policy_error};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = ContentPolicyInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(content_policy_error(input)).map_err(|error| {
            Box::new(crate::helpers::HelperError {
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

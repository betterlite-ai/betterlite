use betterlite_openai::compatible_line_stream::{StreamInput, run_stream};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = StreamInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(run_stream(input)).map_err(|error| {
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

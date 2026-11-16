use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_streaming::stream_tool_assembly::records::AssemblyInput;
use betterlite_streaming::stream_tool_assembly::{AssemblyError, assemble};

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = AssemblyInput;

    fn run(input: &Self::Input) -> HelperResult {
        let output = assemble(input).map_err(|error| {
            Box::new(HelperError {
                class: error_class(&error).to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        serde_json::to_value(output).map_err(|error| {
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

const fn error_class(error: &AssemblyError) -> &str {
    match error {
        AssemblyError::EmptyFunctionInput | AssemblyError::MissingFirstChoice => "IndexError",
        AssemblyError::MissingFirstFunction | AssemblyError::MissingFunctionArguments => {
            "AttributeError"
        }
        AssemblyError::InvalidArguments => "TypeError",
    }
}

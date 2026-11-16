use betterlite_core::api::upload_filename;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) filename: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        upload_filename::safe_filename(&input.filename)
            .map(serde_json::Value::String)
            .map_err(|error| {
                Box::new(HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })
    }
}

use betterlite_models::model_metadata_projection::project;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    key: String,
    model: String,
    provider: String,
    entry: Box<RawValue>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let projected = project(&input.key, &input.model, &input.provider, &input.entry)
            .map_err(|error| helper_error(error.to_string()))?;
        serde_json::from_str(projected.get()).map_err(|error| helper_error(error.to_string()))
    }
}

fn helper_error(message: String) -> Box<HelperError> {
    let error = HelperError {
        class: "ProjectionError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

mod tests;

use betterlite_models::model_metadata_projection::project;
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let projected = project(&input.key, &input.model, &input.provider, &input.entry)
            .map_err(|error| helper_error(error.to_string()))?;
        serde_json::from_str(projected.get()).map_err(|error| helper_error(error.to_string()))
    }
}

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
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

use betterlite_models::price_fields::PriceFields;
use serde_json::Value;

pub(crate) struct Op;

fn project(parsed: &PriceFields) -> Result<Value, Box<crate::helpers::HelperError>> {
    let value = serde_json::to_value(parsed).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "Serialize".to_owned(), status: None, error_type: None, param: None,
            code: None, message: error.to_string(),
        })
    })?;
    Ok(value)
}

impl crate::helpers::Helper for Op {
    type Input = PriceFields;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        project(input)
    }
}

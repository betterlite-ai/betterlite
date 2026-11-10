use serde::de::DeserializeOwned;

use crate::error::HelperError;

pub type HelperResult = Result<serde_json::Value, Box<HelperError>>;

pub trait Helper {
    type Input: DeserializeOwned;

    fn run(input: &Self::Input) -> HelperResult;

    fn decode_and_run(input: &serde_json::Value) -> HelperResult {
        let decoded = serde_json::from_value(input.clone()).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        Self::run(&decoded)
    }

    fn check(input: &serde_json::Value) -> Result<(), String> {
        serde_json::from_value::<Self::Input>(input.clone())
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn not_built(operation: &str) -> HelperResult {
        Err(Box::new(HelperError {
            class: "NotBuilt".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the {operation} helper is not built yet"),
        }))
    }
}

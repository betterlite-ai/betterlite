use betterlite_db::database_failures::metrics::{DbEventMetadata, safe_db_event_metadata};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    values: Vec<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let metadata: Vec<Option<DbEventMetadata>> = input
            .values
            .iter()
            .map(|value| safe_db_event_metadata(Some(value)))
            .collect();
        serde_json::to_value(metadata).map_err(|source| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

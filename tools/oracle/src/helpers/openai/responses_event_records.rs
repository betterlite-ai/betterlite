use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_event_records::wire::decode_responses_event_record;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    events: Vec<Extra>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let records = input
            .events
            .iter()
            .map(decode_event)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(serde_json::Value::Array(records))
    }
}

fn decode_event(input: &Extra) -> HelperResult {
    let bytes = encode_input(input)?;
    let record = decode_responses_event_record(
        &bytes,
        WireDecodeLimits {
            max_json_bytes: 1_000_000,
            max_event_bytes: 1_000_000,
            max_items: 10_000,
        },
    )
    .map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let value = serde_json::to_value(record).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(value)
}

fn encode_input(input: &Extra) -> Result<Vec<u8>, Box<HelperError>> {
    serde_json::to_vec(input).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

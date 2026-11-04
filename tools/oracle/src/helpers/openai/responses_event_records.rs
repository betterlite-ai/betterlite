use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_openai::api::responses_event_records::wire::{
    ResponsesAPIStreamingResponse, decode_responses_event_record,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
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
        let fields = match record {
            ResponsesAPIStreamingResponse::Known { fields, .. }
            | ResponsesAPIStreamingResponse::Generic { fields, .. } => fields,
        };
        let mut value = serde_json::to_value(fields).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        normalize_response(&mut value)?;
        Ok(value)
    }
}

fn encode_input(input: &serde_json::Value) -> Result<Vec<u8>, Box<HelperError>> {
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

fn normalize_response(value: &mut serde_json::Value) -> Result<(), Box<HelperError>> {
    if let Some(response) = value.get_mut("response") {
        let decoded =
            serde_json::from_value::<ResponsesApiResponse>(response.clone()).map_err(|error| {
                Box::new(HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        *response = serde_json::to_value(decoded).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    }
    Ok(())
}

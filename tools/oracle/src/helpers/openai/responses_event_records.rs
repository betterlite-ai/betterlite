pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let bytes = serde_json::to_vec(input).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let record =
            betterlite_openai::api::responses_event_records::wire::decode_responses_event_record(
                &bytes,
                betterlite_openai::api::responses_shapes::wire::WireDecodeLimits {
                    max_json_bytes: 1_000_000,
                    max_event_bytes: 1_000_000,
                    max_items: 10_000,
                },
            )
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        match record {
            betterlite_openai::api::responses_event_records::wire::ResponsesAPIStreamingResponse::Known { fields, .. }
            | betterlite_openai::api::responses_event_records::wire::ResponsesAPIStreamingResponse::Generic { fields, .. } => {
                let mut value = serde_json::to_value(fields).map_err(|error| Box::new(crate::helpers::HelperError {
                    class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string(),
                }))?;
                if let Some(response) = value.get_mut("response") {
                    let decoded = serde_json::from_value::<betterlite_openai::api::responses_objects::ResponsesApiResponse>(response.clone())
                        .map_err(|error| Box::new(crate::helpers::HelperError {
                            class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string(),
                        }))?;
                    *response = serde_json::to_value(decoded).map_err(|error| Box::new(crate::helpers::HelperError {
                        class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string(),
                    }))?;
                }
                Ok(value)
            }
        }
    }
}

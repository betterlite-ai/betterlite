pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = crate::websocket_driver::service::ServiceInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = crate::websocket_driver::service::Services::default()
            .run_service(input.service, &input.input)
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: "ServiceError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        serde_json::to_value(output).map_err(|error| {
            Box::new(crate::helpers::HelperError {
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

pub(crate) struct Op;

fn service_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl crate::helpers::Helper for Op {
    type Input = crate::websocket_driver::service::ServiceInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = crate::websocket_driver::service::Services::default()
            .run_service(input.service, &input.input)
            .map_err(|error| service_error("ServiceError", error.to_string()))?;
        serde_json::to_value(output)
            .map_err(|error| service_error("SerializationError", error.to_string()))
    }
}

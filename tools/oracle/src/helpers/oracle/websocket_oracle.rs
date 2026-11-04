#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    service: crate::websocket_driver::ServiceTag,
    input: serde_json::Value,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        crate::websocket_driver::run_service(input.service, &input.input).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ServiceError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

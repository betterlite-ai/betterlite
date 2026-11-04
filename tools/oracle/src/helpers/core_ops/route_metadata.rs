#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: String,
    #[serde(default)]
    pub(crate) route: Option<String>,
    #[serde(default)]
    pub(crate) call_type: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.function.as_str() {
            "full_table" => betterlite_core::routes::route_catalog()
                .and_then(|catalog| serde_json::to_value(catalog).map_err(Into::into))
                .map_err(route_metadata_error),
            "get_call_types" => {
                let route = input.route.as_deref().unwrap_or_default();
                betterlite_core::routes::get_call_types_for_route(route)
                    .and_then(|types| serde_json::to_value(types).map_err(Into::into))
                    .map_err(route_metadata_error)
            }
            "get_routes" => {
                let encoded = serde_json::to_value(input.call_type.as_deref().unwrap_or_default())
                    .map_err(route_metadata_error)?;
                let call_type = serde_json::from_value(encoded).map_err(route_metadata_error)?;
                betterlite_core::routes::get_routes_for_call_type(call_type)
                    .and_then(|routes| serde_json::to_value(routes).map_err(Into::into))
                    .map_err(route_metadata_error)
            }
            _ => Err(route_metadata_error(std::io::Error::other(
                "unknown function",
            ))),
        }
    }
}

fn route_metadata_error<E: std::fmt::Display>(error: E) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

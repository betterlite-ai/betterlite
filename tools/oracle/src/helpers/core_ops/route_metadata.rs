use std::fmt::Display;

use betterlite_core::routes::{CallType, RouteMetadataError};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    FullTable,
    GetCallTypes { route: String },
    GetRoutes { call_type: CallType },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::FullTable => betterlite_core::routes::catalog::RouteCatalog::parse(
                betterlite_core::routes::catalog::CATALOG,
            )
            .and_then(|catalog| serde_json::to_value(catalog).map_err(RouteMetadataError::Fixture))
            .map_err(route_metadata_error),
            Input::GetCallTypes { route } => {
                match betterlite_core::routes::get_call_types_for_route(route) {
                    Ok(Some(types)) => serde_json::to_value(serde_json::json!({
                        "call_types": types
                    }))
                    .map_err(RouteMetadataError::Fixture)
                    .map_err(route_metadata_error),
                    Ok(None) => Ok(serde_json::json!({"call_types": null})),
                    Err(error) => Err(route_metadata_error(error)),
                }
            }
            Input::GetRoutes { call_type } => serde_json::to_value(
                betterlite_core::routes::get_routes_for_call_type(*call_type),
            )
            .map_err(RouteMetadataError::Fixture)
            .map_err(route_metadata_error),
        }
    }
}

fn route_metadata_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

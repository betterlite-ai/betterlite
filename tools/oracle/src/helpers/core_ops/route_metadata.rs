use std::fmt::Display;

use betterlite_core::routes::{
    CallType,
    catalog::{CATALOG, RouteCatalog},
};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    FullTable,
    GetCallTypes { route: String },
    GetRoutes { call_type: CallType },
}

#[derive(serde::Serialize)]
struct CallTypesOutput<'catalog> {
    call_types: Option<&'catalog [CallType]>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let catalog = RouteCatalog::parse(CATALOG).map_err(route_metadata_error)?;
        let output = match input {
            Input::FullTable => serde_json::to_value(&catalog),
            Input::GetCallTypes { route } => serde_json::to_value(CallTypesOutput {
                call_types: catalog
                    .call_types_for(route)
                    .map_err(route_metadata_error)?,
            }),
            Input::GetRoutes { call_type } => serde_json::to_value(catalog.routes_for(*call_type)),
        };
        output.map_err(route_metadata_error)
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

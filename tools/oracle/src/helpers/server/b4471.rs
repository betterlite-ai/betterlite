use betterlite_backend::BACKEND_ROUTES;
use betterlite_gateway::routes::GATEWAY_ROUTES;
use betterlite_gateway::serve::matches_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    component: Component,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Component {
    Gateway,
    Backend,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let routes = match input.component {
            Component::Gateway => GATEWAY_ROUTES,
            Component::Backend => BACKEND_ROUTES,
        };
        serde_json::to_value(matches_path(&input.path, routes))
            .map_err(|error| helper_error(error.to_string()))
    }
}

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

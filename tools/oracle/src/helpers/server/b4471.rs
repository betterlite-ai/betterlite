use betterlite_backend::BACKEND_ROUTES;
use betterlite_gateway::routes::GATEWAY_ROUTES;
use betterlite_server::server::filter::keeps_route;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    component: Component,
    paths: Vec<String>,
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
        let allowed: Vec<bool> = input
            .paths
            .iter()
            .map(|path| keeps_route(path, routes))
            .collect();
        serde_json::to_value(allowed).map_err(|error| helper_error(error.to_string()))
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

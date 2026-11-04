use betterlite_server::route_inspection::{
    NamedEndpoint, Route, RouteInspectionError, get_app_routes, get_routes_for_mounted_app,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GetAppRoutes {
        route: Route,
        endpoint_name: Option<String>,
    },
    GetRoutesForMountedApp {
        route: Route,
    },
    MountedSubRouteInfo {
        mount_path: String,
        route: Route,
    },
    SafeGetEndpointName {
        endpoint_name: Option<String>,
        class_name: Option<String>,
        fail: bool,
    },
}

struct Endpoint(Option<String>);

impl NamedEndpoint for Endpoint {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.0.as_deref())
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::GetAppRoutes {
                route,
                endpoint_name,
            } => json!(get_app_routes(route, &Endpoint(endpoint_name.clone()))),
            Input::GetRoutesForMountedApp { route } => json!(get_routes_for_mounted_app(route)),
            Input::MountedSubRouteInfo { mount_path, route } => {
                let mount = Route {
                    path: Some(mount_path.clone()),
                    methods: None,
                    name: None,
                    endpoint: None,
                    app: Some(vec![route.clone()]),
                };
                get_routes_for_mounted_app(&mount)
                    .into_iter()
                    .next()
                    .map_or(Value::Null, |info| json!(info))
            }
            Input::SafeGetEndpointName {
                endpoint_name,
                class_name,
                fail,
            } => {
                if *fail {
                    Value::Null
                } else {
                    json!(endpoint_name.as_ref().or(class_name.as_ref()))
                }
            }
        };
        Ok(output)
    }
}

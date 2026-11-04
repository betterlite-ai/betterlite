use betterlite_server::route_inspection::{
    NamedEndpoint, Route, RouteInspectionError, get_app_routes, get_routes_for_mounted_app,
    safe_get_endpoint_name,
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

struct Endpoint {
    name: Option<String>,
    class_name: Option<String>,
    fail: bool,
}

impl NamedEndpoint for Endpoint {
    fn endpoint_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        if self.fail {
            Err(RouteInspectionError::EndpointName)
        } else {
            Ok(self.name.as_deref())
        }
    }

    fn class_name(&self) -> Result<Option<&str>, RouteInspectionError> {
        Ok(self.class_name.as_deref())
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::GetAppRoutes {
                route,
                endpoint_name,
            } => json!(get_app_routes(
                route,
                &Endpoint {
                    name: endpoint_name.clone(),
                    class_name: None,
                    fail: false,
                }
            )),
            Input::GetRoutesForMountedApp { route } => json!(get_routes_for_mounted_app(route)),
            Input::MountedSubRouteInfo { mount_path, route } => {
                let mount = Route {
                    path: Some(mount_path.clone()),
                    methods: None,
                    name: None,
                    endpoint: None,
                    app_name: None,
                    app: Some(vec![route.clone()]),
                };
                let result = get_routes_for_mounted_app(&mount)
                    .into_iter()
                    .next()
                    .map_or(Value::Null, |info| json!(info));
                json!({"result": result})
            }
            Input::SafeGetEndpointName {
                endpoint_name,
                class_name,
                fail,
            } => json!({
                "result": safe_get_endpoint_name(
                    "oracle endpoint",
                    &Endpoint {
                        name: endpoint_name.clone(),
                        class_name: class_name.clone(),
                        fail: *fail,
                    },
                )
            }),
        };
        Ok(output)
    }
}

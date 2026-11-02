use std::collections::BTreeSet;

use betterlite_auth::pass_through_registry::{PassThroughMetadata, PassThroughRouteType};
use betterlite_server::passthrough_route_registry::listing::registered_route_keys;
use betterlite_server::passthrough_route_registry::removal::remove_endpoint_routes;
use betterlite_server::passthrough_route_registry::route_table::RouteTable;

use crate::helpers::HelperResult;

pub(crate) struct Op;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Collision(CollisionRequest),
    Remove(RemoveRequest),
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollisionRequest {
    existing_routes: Vec<ExistingRoute>,
    new_path: String,
    new_methods: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExistingRoute {
    path: String,
    methods: Vec<String>,
    #[serde(default)]
    lazy_owned: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoveRequest {
    registrations: Vec<RegistrationInput>,
    openai_routes: Vec<String>,
    endpoint_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationInput {
    key: String,
    endpoint_id: String,
    route_type: RouteTypeInput,
    path: String,
    #[serde(default)]
    methods: Vec<String>,
    auth: Option<bool>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouteTypeInput {
    Exact,
    Subpath,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            Input::Collision(request) => collision(request),
            Input::Remove(request) => remove(request),
        })
    }
}

fn collision(request: &CollisionRequest) -> serde_json::Value {
    let mut table = RouteTable::default();
    let mut existing_ids = Vec::new();
    for route in &request.existing_routes {
        let methods: BTreeSet<String> = route.methods.iter().cloned().collect();
        existing_ids.push(table.install(route.path.clone(), methods, route.lazy_owned));
    }
    let new_methods: BTreeSet<String> = request.new_methods.iter().cloned().collect();
    let added = table.add_route_if_not_exists(request.new_path.clone(), new_methods);
    let mut route_order = Vec::new();
    for id in table.order() {
        let label = existing_ids
            .iter()
            .position(|existing| *existing == id)
            .map_or_else(|| "new".to_owned(), |index| format!("existing[{index}]"));
        route_order.push(label);
    }
    serde_json::json!({"added": added, "route_order": route_order})
}

fn remove(request: &RemoveRequest) -> serde_json::Value {
    let mut registrations: Vec<(String, PassThroughMetadata)> =
        request.registrations.iter().map(to_registration).collect();
    let mut openai_routes = request.openai_routes.clone();
    remove_endpoint_routes(&mut registrations, &mut openai_routes, &request.endpoint_id);
    serde_json::json!({
        "registered_keys": registered_route_keys(&registrations),
        "openai_routes": openai_routes,
    })
}

fn to_registration(registration: &RegistrationInput) -> (String, PassThroughMetadata) {
    let route_type = match registration.route_type {
        RouteTypeInput::Exact => PassThroughRouteType::Exact,
        RouteTypeInput::Subpath => PassThroughRouteType::Subpath,
    };
    (
        registration.key.clone(),
        PassThroughMetadata {
            endpoint_id: registration.endpoint_id.clone(),
            route_type,
            path: registration.path.clone(),
            methods: registration.methods.clone(),
            auth: registration.auth,
        },
    )
}

use serde::Deserialize;
use serde_json::{Map, Value};

use betterlite_auth::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    root_path: String,
    registrations: Vec<Registration>,
    queries: Vec<Query>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    key: String,
    endpoint_id: String,
    route_type: RouteType,
    path: String,
    #[serde(default)]
    methods: Vec<String>,
    auth: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouteType {
    Exact,
    Subpath,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    route: String,
    method: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let registrations = input.registrations.iter().map(to_registration).collect();
        let registry = PassThroughRegistry {
            root_path: input.root_path.clone(),
            registrations,
        };
        let answers = input
            .queries
            .iter()
            .map(|query| {
                let selected_metadata =
                    registry.get_registered(&query.route, query.method.as_deref());
                let metadata = selected_metadata.map(metadata_value);
                serde_json::json!({
                    "normalized": registry.lookup_route(&query.route),
                    "registered": registry.is_registered(&query.route),
                    "metadata": metadata,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_value(answers).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn to_registration(registration: &Registration) -> (String, PassThroughMetadata) {
    let route_type = match registration.route_type {
        RouteType::Exact => PassThroughRouteType::Exact,
        RouteType::Subpath => PassThroughRouteType::Subpath,
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

fn metadata_value(metadata: &PassThroughMetadata) -> Value {
    let mut value = Map::new();
    value.insert(
        "endpoint_id".to_owned(),
        Value::String(metadata.endpoint_id.clone()),
    );
    value.insert("path".to_owned(), Value::String(metadata.path.clone()));
    value.insert("methods".to_owned(), serde_json::json!(metadata.methods));
    value.insert(
        "route_type".to_owned(),
        serde_json::json!(match metadata.route_type {
            PassThroughRouteType::Exact => "exact",
            PassThroughRouteType::Subpath => "subpath",
        }),
    );
    if let Some(auth) = metadata.auth {
        value.insert("auth".to_owned(), Value::Bool(auth));
    }
    Value::Object(value)
}

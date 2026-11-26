use std::collections::BTreeMap;

use betterlite_auth::pass_through_registry::{
    PassThroughMetadata, PassThroughRegistry, PassThroughRouteType,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::virtual_key_routes::caller::{
    RouteCaller, check_passthrough_route_access, jwt_team_grant_pass_through,
};
use betterlite_auth::virtual_key_routes::pass_through::is_protected_pass_through;
use betterlite_auth::virtual_key_routes::route_checks::{CalledRoute, check_virtual_key_route};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

pub(crate) struct Op;

// the method list LiteLLM stores when a registered pass-through names none
const DEFAULT_METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "PATCH"];

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    ShouldCallRoute,
    PassthroughAccess,
    AuthEnforced,
    JwtGrant,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    route: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    key: Option<KeyFields>,
    #[serde(default)]
    registrations: Vec<Registration>,
    #[serde(default)]
    team_allowed_routes: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct KeyFields {
    allowed_routes: Option<Vec<JsonData>>,
    metadata: Option<BTreeMap<String, JsonData>>,
    team_metadata: Option<BTreeMap<String, JsonData>>,
}

// one entry of the in-memory pass-through registry, as LiteLLM's registration writes it
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    endpoint_id: String,
    route_type: RouteType,
    path: String,
    #[serde(default)]
    methods: Option<Vec<String>>,
    #[serde(default)]
    auth: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RouteType {
    Exact,
    Subpath,
}

impl RouteType {
    const fn key_name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Subpath => "subpath",
        }
    }

    const fn registry_type(self) -> PassThroughRouteType {
        match self {
            Self::Exact => PassThroughRouteType::Exact,
            Self::Subpath => PassThroughRouteType::Subpath,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let key = caller_key(input.key.as_ref())?;
        let caller = RouteCaller::of_key(&key);
        let registry = registry_of(&input.registrations);
        let method = input.method.as_deref().map(str::to_uppercase);
        let called = CalledRoute {
            route: &input.route,
            method: method.as_deref(),
        };
        Ok(match input.function {
            Function::ShouldCallRoute => {
                match check_virtual_key_route(called, &caller, &registry) {
                    Ok(()) => answer(true, None),
                    Err(exception) => answer(false, Some(exception.as_ref())),
                }
            }
            Function::PassthroughAccess => {
                answer(check_passthrough_route_access(&input.route, &caller), None)
            }
            Function::AuthEnforced => answer(
                is_protected_pass_through(&input.route, method.as_deref(), &registry),
                None,
            ),
            Function::JwtGrant => answer(
                jwt_team_grant_pass_through(&input.route, &input.team_allowed_routes),
                None,
            ),
        })
    }
}

fn caller_key(key_fields: Option<&KeyFields>) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut dict = BTreeMap::new();
    if let Some(fields) = key_fields {
        if let Some(routes) = &fields.allowed_routes {
            dict.insert("allowed_routes".to_owned(), JsonData::Array(routes.clone()));
        }
        for (name, value) in [
            ("metadata", fields.metadata.as_ref()),
            ("team_metadata", fields.team_metadata.as_ref()),
        ] {
            if let Some(entries) = value {
                dict.insert(name.to_owned(), JsonData::Object(entries.clone()));
            }
        }
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(dict)).or_value_error()
}

fn registry_of(registrations: &[Registration]) -> PassThroughRegistry {
    PassThroughRegistry {
        root_path: String::new(),
        registrations: registrations
            .iter()
            .map(|registration| {
                (
                    registration_key(registration),
                    registration_metadata(registration),
                )
            })
            .collect(),
    }
}

fn registration_key(registration: &Registration) -> String {
    let mut methods = registration_methods(registration);
    methods.sort();
    format!(
        "{}:{}:{}:{}",
        registration.endpoint_id,
        registration.route_type.key_name(),
        registration.path,
        methods.join(",")
    )
}

fn registration_metadata(registration: &Registration) -> PassThroughMetadata {
    PassThroughMetadata {
        endpoint_id: registration.endpoint_id.clone(),
        route_type: registration.route_type.registry_type(),
        path: registration.path.clone(),
        methods: registration_methods(registration),
        auth: registration.auth,
    }
}

fn registration_methods(registration: &Registration) -> Vec<String> {
    match &registration.methods {
        Some(methods) if !methods.is_empty() => methods.clone(),
        Some(_) | None => DEFAULT_METHODS.map(str::to_owned).to_vec(),
    }
}

fn answer(allowed: bool, denial: Option<&ProxyException>) -> Value {
    let error = denial.map_or(Value::Null, |exception| {
        json!({
            "status": exception.code.parse::<u16>().unwrap_or_default(),
            "error_type": exception.error_type.as_str(),
            "param": &exception.param,
            "code": exception.code.as_str(),
        })
    });
    json!({"allowed": allowed, "error": error})
}

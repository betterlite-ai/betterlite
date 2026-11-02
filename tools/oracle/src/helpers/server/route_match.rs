use crate::helpers::Helper;
use betterlite_auth::route_match::classifier::{
    is_azure_openai_route, is_info_route, is_llm_api_route, is_management_route,
    is_mcp_discovery_route,
};
use betterlite_auth::route_match::groups::get_actual_routes;
use betterlite_auth::route_match::method::{MethodAccess, get_request_method};
use betterlite_auth::route_match::templates::{
    RouteValue, check_route_access, route_matches_allowed_route, route_matches_pattern,
    route_matches_wildcard_pattern,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: String,
    route: Option<Value>,
    pattern: Option<String>,
    allowed_route: Option<String>,
    allowed_routes: Option<Vec<String>>,
    method: Option<String>,
    routes: Option<Vec<String>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.function.as_str() {
            "get_actual_routes" => actual_routes(input),
            "get_request_method" => request_method(input),
            "is_llm_api_route"
            | "is_azure_openai_route"
            | "is_management_route"
            | "is_info_route"
            | "is_mcp_discovery_route" => Ok(classifier(input)),
            _ => predicate(input),
        }
    }
}

fn classifier(input: &Input) -> Value {
    let route = input.route.as_ref().and_then(Value::as_str);
    let value = match input.function.as_str() {
        "is_llm_api_route" => {
            is_llm_api_route(&route.map_or(RouteValue::NonString, RouteValue::Text))
        }
        "is_azure_openai_route" => {
            is_azure_openai_route(&route.map_or(RouteValue::NonString, RouteValue::Text))
        }
        "is_management_route" => route.is_some_and(is_management_route),
        "is_info_route" => route.is_some_and(is_info_route),
        _ => route.is_some_and(|path| is_mcp_discovery_route(path, input.method.as_deref())),
    };
    Value::Bool(value)
}

fn predicate(input: &Input) -> crate::helpers::HelperResult {
    let route = input.route.as_ref().and_then(Value::as_str);
    let value = match input.function.as_str() {
        "route_matches_pattern" => route.is_some_and(|path| {
            input
                .pattern
                .as_deref()
                .is_some_and(|pattern| route_matches_pattern(&RouteValue::Text(path), pattern))
        }),
        "is_wildcard_pattern" => input
            .pattern
            .as_deref()
            .is_some_and(|pattern| matches!(pattern.chars().last(), Some('*'))),
        "route_matches_wildcard_pattern" => route.is_some_and(|path| {
            input
                .pattern
                .as_deref()
                .is_some_and(|pattern| route_matches_wildcard_pattern(path, pattern))
        }),
        "route_matches_allowed_route" => route.is_some_and(|path| {
            input
                .allowed_route
                .as_deref()
                .is_some_and(|allowed| route_matches_allowed_route(path, allowed))
        }),
        "check_route_access" => route.is_some_and(|path| {
            input.allowed_routes.as_deref().is_some_and(|allowed| {
                check_route_access(
                    path,
                    &allowed.iter().map(String::as_str).collect::<Vec<_>>(),
                )
            })
        }),
        _ => return Op::not_built("route_match function"),
    };
    Ok(Value::Bool(value))
}

fn actual_routes(input: &Input) -> crate::helpers::HelperResult {
    let routes = input.routes.as_deref().unwrap_or_default();
    let references: Vec<&str> = routes.iter().map(String::as_str).collect();
    serde_json::to_value(get_actual_routes(&references)).map_err(|error| {
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

fn request_method(input: &Input) -> crate::helpers::HelperResult {
    let access = input
        .method
        .as_deref()
        .map_or(MethodAccess::NoRequest, MethodAccess::Text);
    let method = get_request_method(access).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "MethodError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    serde_json::to_value(method).map_err(|error| {
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

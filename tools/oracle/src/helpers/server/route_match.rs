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
        let route = input.route.as_ref().and_then(Value::as_str);
        let result = match input.function.as_str() {
            "get_actual_routes" => {
                let routes = input.routes.as_deref().unwrap_or_default();
                let references: Vec<&str> = routes.iter().map(String::as_str).collect();
                serde_json::to_value(get_actual_routes(&references))
                    .map_err(|error| helper_error("SerializationError", error.to_string()))
            }
            "get_request_method" => {
                let access = input
                    .method
                    .as_deref()
                    .map_or(MethodAccess::NoRequest, MethodAccess::Text);
                get_request_method(access)
                    .map_err(|error| helper_error("MethodError", error.to_string()))
                    .and_then(|method| {
                        serde_json::to_value(method)
                            .map_err(|error| helper_error("SerializationError", error.to_string()))
                    })
            }
            "is_llm_api_route" => Ok(Value::Bool(is_llm_api_route(
                &route.map_or(RouteValue::NonString, RouteValue::Text),
            ))),
            "is_azure_openai_route" => Ok(Value::Bool(is_azure_openai_route(
                &route.map_or(RouteValue::NonString, RouteValue::Text),
            ))),
            "is_management_route" => Ok(Value::Bool(route.is_some_and(is_management_route))),
            "is_info_route" => Ok(Value::Bool(route.is_some_and(is_info_route))),
            "is_mcp_discovery_route" => {
                Ok(Value::Bool(route.is_some_and(|path| {
                    is_mcp_discovery_route(path, input.method.as_deref())
                })))
            }
            "route_matches_pattern" => Ok(Value::Bool(route.is_some_and(|path| {
                input
                    .pattern
                    .as_deref()
                    .is_some_and(|pattern| route_matches_pattern(&RouteValue::Text(path), pattern))
            }))),
            "is_wildcard_pattern" => {
                Ok(Value::Bool(input.pattern.as_deref().is_some_and(
                    |pattern| matches!(pattern.chars().last(), Some('*')),
                )))
            }
            "route_matches_wildcard_pattern" => Ok(Value::Bool(route.is_some_and(|path| {
                input
                    .pattern
                    .as_deref()
                    .is_some_and(|pattern| route_matches_wildcard_pattern(path, pattern))
            }))),
            "route_matches_allowed_route" => Ok(Value::Bool(route.is_some_and(|path| {
                input
                    .allowed_route
                    .as_deref()
                    .is_some_and(|allowed| route_matches_allowed_route(path, allowed))
            }))),
            "check_route_access" => Ok(Value::Bool(route.is_some_and(|path| {
                input.allowed_routes.as_deref().is_some_and(|allowed| {
                    check_route_access(
                        path,
                        &allowed.iter().map(String::as_str).collect::<Vec<_>>(),
                    )
                })
            }))),
            _ => return Op::not_built("route_match function"),
        };
        result
    }
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

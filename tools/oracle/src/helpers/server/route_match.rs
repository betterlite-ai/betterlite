use crate::helpers::Helper;
use betterlite_auth::route_match::classifier::{
    is_azure_openai_route, is_llm_api_route, is_mcp_discovery_route,
};
use betterlite_auth::route_match::groups::get_actual_routes;
use betterlite_auth::route_match::method::{MethodAccess, get_request_method};
use betterlite_auth::route_match::templates::{
    RouteValue, check_route_access, route_matches_allowed_route, route_matches_pattern,
    route_matches_wildcard_pattern,
};
use betterlite_auth::{auxiliary_routes, management_routes};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    RouteLists(RouteListInput),
    Methods(MethodInput),
    RouteChecks(RouteCheckInput),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum RouteListInput {
    #[serde(rename = "get_actual_routes")]
    GetActualRoutes { routes: Option<Vec<String>> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum MethodInput {
    #[serde(rename = "get_request_method")]
    GetRequestMethod { method: Option<String> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum RouteCheckInput {
    #[serde(rename = "is_llm_api_route")]
    LlmApi { route: Option<Value> },
    #[serde(rename = "is_azure_openai_route")]
    AzureOpenai { route: Option<Value> },
    #[serde(rename = "is_management_route")]
    Management { route: Option<Value> },
    #[serde(rename = "is_info_route")]
    Info { route: Option<Value> },
    #[serde(rename = "is_mcp_discovery_route")]
    IsMcpDiscoveryRoute {
        route: Option<Value>,
        method: Option<String>,
    },
    #[serde(rename = "route_matches_pattern")]
    RouteMatchesPattern {
        route: Option<Value>,
        pattern: Option<String>,
    },
    #[serde(rename = "is_wildcard_pattern")]
    IsWildcardPattern { pattern: Option<String> },
    #[serde(rename = "route_matches_wildcard_pattern")]
    RouteMatchesWildcardPattern {
        route: Option<Value>,
        pattern: Option<String>,
    },
    #[serde(rename = "route_matches_allowed_route")]
    RouteMatchesAllowedRoute {
        route: Option<Value>,
        allowed_route: Option<String>,
    },
    #[serde(rename = "check_route_access")]
    CheckRouteAccess {
        route: Option<Value>,
        allowed_routes: Option<Vec<String>>,
    },
}

#[derive(Clone, Copy)]
enum Classifier {
    Llm,
    Azure,
    Management,
    Info,
}

#[derive(Clone, Copy)]
enum PatternMatcher {
    Template,
    Wildcard,
}

enum AllowList<'routes> {
    One(&'routes str),
    Many(&'routes [String]),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::RouteLists(operation) => operation.run(),
            Input::Methods(operation) => operation.run(),
            Input::RouteChecks(operation) => Ok(Value::Bool(operation.result())),
        }
    }
}

impl RouteListInput {
    fn run(&self) -> crate::helpers::HelperResult {
        match self {
            Self::GetActualRoutes { routes } => actual_routes(routes.as_deref()),
        }
    }
}

impl MethodInput {
    fn run(&self) -> crate::helpers::HelperResult {
        match self {
            Self::GetRequestMethod { method } => request_method(method.as_deref()),
        }
    }
}

impl RouteCheckInput {
    fn result(&self) -> bool {
        match self {
            Self::LlmApi { route } => classify(Classifier::Llm, route_text(route.as_ref())),
            Self::AzureOpenai { route } => classify(Classifier::Azure, route_text(route.as_ref())),
            Self::Management { route } => {
                classify(Classifier::Management, route_text(route.as_ref()))
            }
            Self::Info { route } => classify(Classifier::Info, route_text(route.as_ref())),
            Self::IsMcpDiscoveryRoute { route, method } => route_text(route.as_ref())
                .is_some_and(|path| is_mcp_discovery_route(path, method.as_deref())),
            Self::RouteMatchesPattern { route, pattern }
            | Self::RouteMatchesWildcardPattern { route, pattern } => route_pattern_match(
                route.as_ref(),
                pattern.as_deref(),
                if matches!(self, Self::RouteMatchesPattern { .. }) {
                    PatternMatcher::Template
                } else {
                    PatternMatcher::Wildcard
                },
            ),
            Self::IsWildcardPattern { pattern } => pattern
                .as_deref()
                .is_some_and(|expression| expression.ends_with('*')),
            Self::RouteMatchesAllowedRoute {
                route,
                allowed_route,
            } => allowed_check(route.as_ref(), allowed_route.as_deref().map(AllowList::One)),
            Self::CheckRouteAccess {
                route,
                allowed_routes,
            } => allowed_check(
                route.as_ref(),
                allowed_routes.as_deref().map(AllowList::Many),
            ),
        }
    }
}

fn route_text(route: Option<&Value>) -> Option<&str> {
    match route {
        Some(Value::String(path)) => Some(path),
        Some(_) | None => None,
    }
}

fn route_pattern_match(
    route: Option<&Value>,
    pattern: Option<&str>,
    matcher: PatternMatcher,
) -> bool {
    route_text(route)
        .zip(pattern)
        .is_some_and(|(path, expression)| match matcher {
            PatternMatcher::Template => route_matches_pattern(&RouteValue::Text(path), expression),
            PatternMatcher::Wildcard => route_matches_wildcard_pattern(path, expression),
        })
}

fn allowed_check(route: Option<&Value>, allowed: Option<AllowList<'_>>) -> bool {
    route_text(route)
        .zip(allowed)
        .is_some_and(|(path, routes)| match routes {
            AllowList::One(allowed_route) => route_matches_allowed_route(path, allowed_route),
            AllowList::Many(allowed_routes) => check_route_access(
                path,
                &allowed_routes
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ),
        })
}

fn classify(classifier: Classifier, route: Option<&str>) -> bool {
    let Some(path) = route else {
        return false;
    };
    match classifier {
        Classifier::Llm => is_llm_api_route(&RouteValue::Text(path)),
        Classifier::Azure => is_azure_openai_route(&RouteValue::Text(path)),
        Classifier::Management => check_route_access(path, &management_routes::management_routes()),
        Classifier::Info => auxiliary_routes::INFO_ROUTES.contains(&path),
    }
}

fn actual_routes(routes: Option<&[String]>) -> crate::helpers::HelperResult {
    let references: Vec<&str> = routes
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect();
    serde_json::to_value(get_actual_routes(&references))
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

fn request_method(method: Option<&str>) -> crate::helpers::HelperResult {
    let access = method.map_or(MethodAccess::NoRequest, MethodAccess::Text);
    let parsed_method = get_request_method(access)
        .map_err(|error| helper_error("MethodError", error.to_string()))?;
    serde_json::to_value(parsed_method)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
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

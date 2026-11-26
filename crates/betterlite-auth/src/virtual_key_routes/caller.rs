use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_routes::wildcard::route_matches_wildcard_pattern;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::route_match::templates::route_matches_allowed_route;

const ALLOWED_PASSTHROUGH_ROUTES: &str = "allowed_passthrough_routes";
const PASSWORD_RESET_REQUIRED: &str = "password_reset_required";

// LiteLLM allows every route when allowed_routes is None, is not a list, or is an empty list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllowedRoutes<'routes> {
    Unset,
    NotAList,
    Listed(&'routes [JsonData]),
}

// the caller facts every route grant reads, without the key itself so a stage can hold them
#[derive(Clone, Copy, Debug)]
pub struct RouteCaller<'caller> {
    pub allowed_routes: AllowedRoutes<'caller>,
    pub metadata: Option<&'caller Payload>,
    pub team_metadata: Option<&'caller Payload>,
    pub jwt_claims: bool,
    pub has_token: bool,
    pub team_id: Option<&'caller str>,
}

impl<'caller> RouteCaller<'caller> {
    #[must_use]
    pub fn of_key(key: &'caller UserAPIKeyAuth) -> Self {
        let token = &key.view.token;
        let allowed_routes = token
            .allowed_routes
            .0
            .as_ref()
            .map_or(AllowedRoutes::Unset, |routes| {
                AllowedRoutes::Listed(routes.as_slice())
            });
        Self {
            allowed_routes,
            metadata: Some(&token.metadata),
            team_metadata: key.view.team_metadata.as_ref(),
            jwt_claims: key.extra.jwt_claims.is_some(),
            has_token: token.token.is_some(),
            team_id: token.team_id.as_deref(),
        }
    }
}

#[must_use]
pub fn password_reset_required(caller: &RouteCaller<'_>) -> bool {
    caller.metadata.is_some_and(|metadata| {
        matches!(
            metadata.0.get(PASSWORD_RESET_REQUIRED),
            Some(JsonData::Bool(true))
        )
    })
}

// the key's truthy list wins; only a missing, null, empty or otherwise falsy key list falls back to the team's
#[must_use]
pub fn check_passthrough_route_access(route: &str, caller: &RouteCaller<'_>) -> bool {
    let key_routes = caller
        .metadata
        .and_then(|payload| payload.0.get(ALLOWED_PASSTHROUGH_ROUTES));
    let team_routes = caller
        .team_metadata
        .and_then(|payload| payload.0.get(ALLOWED_PASSTHROUGH_ROUTES));
    let granted = key_routes
        .filter(|routes| is_nonempty(routes))
        .or_else(|| team_routes.filter(|routes| is_nonempty(routes)));
    granted.is_some_and(|routes| passthrough_routes_allow(route, routes))
}

#[must_use]
pub const fn jwt_team_allowed_routes<'settings>(
    caller: &RouteCaller<'_>,
    settings: &'settings [String],
) -> &'settings [String] {
    if caller.jwt_claims && !caller.has_token && caller.team_id.is_some() {
        settings
    } else {
        &[]
    }
}

#[must_use]
pub fn jwt_team_grant_pass_through(route: &str, team_allowed_routes: &[String]) -> bool {
    team_allowed_routes.iter().any(|allowed| {
        let path = allowed.trim_end_matches('*').trim_matches('/');
        !path.is_empty() && route_matches_wildcard_pattern(route, allowed)
    })
}

// Python iterates whatever the truthy value is: strings by character, objects by key
fn passthrough_routes_allow(route: &str, routes: &JsonData) -> bool {
    match routes {
        JsonData::Array(items) => items
            .iter()
            .filter_map(text_entry)
            .any(|allowed| route_matches_allowed_route(route, allowed)),
        JsonData::String(text) => text
            .chars()
            .any(|character| route_matches_allowed_route(route, &character.to_string())),
        JsonData::Object(entries) => entries
            .keys()
            .any(|allowed| route_matches_allowed_route(route, allowed)),
        // a bare bool or number raises in LiteLLM, and the stage answers with the pass-through denial instead
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) => false,
    }
}

fn is_nonempty(value: &JsonData) -> bool {
    match value {
        JsonData::Null => false,
        JsonData::Bool(flag) => *flag,
        // Python's bool() is false for both zeroes, and true for NaN
        JsonData::Number(number) => number
            .as_f64()
            .is_none_or(|magnitude| magnitude.abs().total_cmp(&0.0_f64).is_ne()),
        JsonData::String(text) => !text.is_empty(),
        JsonData::Array(items) => !items.is_empty(),
        JsonData::Object(entries) => !entries.is_empty(),
    }
}

pub(crate) const fn text_entry(value: &JsonData) -> Option<&str> {
    match value {
        JsonData::String(text) => Some(text.as_str()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

mod tests;

use betterlite_auth::route_match::templates::route_matches_wildcard_pattern;

use crate::context::PolicyMatchContext;
use crate::values::PolicyScope;

#[must_use]
pub fn matches_pattern(value: Option<&str>, patterns: &[String]) -> bool {
    value.map_or_else(
        || patterns.iter().any(|pattern| pattern == "*"),
        |present| {
            patterns
                .iter()
                .any(|pattern| route_matches_wildcard_pattern(present, pattern))
        },
    )
}

#[must_use]
pub fn scope_matches(scope: &PolicyScope, context: &PolicyMatchContext) -> bool {
    let teams = scope.get_teams();
    if !matches_pattern(context.team_alias.as_deref(), &teams) {
        return false;
    }
    let keys = scope.get_keys();
    if !matches_pattern(context.key_alias.as_deref(), &keys) {
        return false;
    }
    let models = scope.get_models();
    if !matches_pattern(context.model.as_deref(), &models) {
        return false;
    }
    let tags = scope.get_tags();
    if tags.is_empty() {
        return true;
    }
    let Some(context_tags) = context.tags.as_ref().filter(|present| !present.is_empty()) else {
        return false;
    };
    context_tags
        .iter()
        .any(|tag| matches_pattern(Some(tag.as_str()), &tags))
}

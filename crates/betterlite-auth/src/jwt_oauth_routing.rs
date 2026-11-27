use std::slice;

use betterlite_config::sections::jwt_settings::records::{OverridePath, RoutingOverride, Selector};

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::decode::unverified_claims;
use crate::jwt_role_claims::fnmatch::RolePattern;

// str.strip() and str.isspace() also count U+001C to U+001F, which char::is_whitespace leaves out
const fn is_strip_whitespace(letter: char) -> bool {
    match letter {
        '\u{1c}'..='\u{1f}' => true,
        _ => letter.is_whitespace(),
    }
}

// the claim values a selector is tested against: a string is one value, a list its string members
fn claim_values<'claims>(
    claims: &'claims TypedValue,
    claim: &str,
    split_space_delimited: bool,
) -> Vec<&'claims str> {
    let Some(value) = claims.json.get(claim) else {
        return Vec::new();
    };
    if let Some(items) = value.as_array() {
        return items.iter().filter_map(|item| item.as_str()).collect();
    }
    let Some(text) = value.as_str() else {
        return Vec::new();
    };
    if !split_space_delimited {
        return vec![text];
    }
    let stripped = text.trim_matches(is_strip_whitespace);
    if !stripped.contains(' ') {
        return vec![text];
    }
    stripped
        .split(' ')
        .filter(|word| !word.is_empty())
        .collect()
}

fn selector_matches(selector: &str, claim: &str, split_space_delimited: bool) -> bool {
    if !selector.contains('*') && !selector.contains('?') {
        return selector == claim;
    }
    // a wildcard may not span whitespace on the claims that never split, so "trusted.*" misses "a b"
    if !split_space_delimited && claim.chars().any(is_strip_whitespace) {
        return false;
    }
    RolePattern::new(selector).matches(claim)
}

#[must_use]
pub fn routing_selector_matches_claim(
    selector: Option<&Selector>,
    claims: &TypedValue,
    claim: &str,
    split_space_delimited: bool,
) -> bool {
    let Some(selector) = selector else {
        return true;
    };
    let selectors: &[String] = match selector {
        Selector::One(one) => slice::from_ref(one),
        Selector::Many(many) => many.as_slice(),
    };
    let values = claim_values(claims, claim, split_space_delimited);
    selectors.iter().any(|candidate| {
        values
            .iter()
            .copied()
            .any(|value| selector_matches(candidate, value, split_space_delimited))
    })
}

#[must_use]
pub fn matches_routing_override(claims: &TypedValue, rule: &RoutingOverride) -> bool {
    routing_selector_matches_claim(Some(&rule.iss), claims, "iss", false)
        && routing_selector_matches_claim(rule.client_id.as_ref(), claims, "client_id", false)
        && routing_selector_matches_claim(rule.scope.as_ref(), claims, "scope", true)
        && routing_selector_matches_claim(rule.aud.as_ref(), claims, "aud", false)
}

// the routing decision reads unverified claims only to pick a validator; the validator still authenticates
#[must_use]
// ast-grep-ignore: no-long-names
pub fn should_route_jwt_to_oauth2_override(
    token: &str,
    overrides: Option<&[RoutingOverride]>,
) -> bool {
    let Some(overrides) = overrides else {
        return false;
    };
    if overrides.is_empty() {
        return false;
    }
    let Some(claims) = unverified_claims(token) else {
        return false;
    };
    for rule in overrides {
        if rule.path == OverridePath::Oauth2 && matches_routing_override(&claims, rule) {
            tracing::debug!("JWT routing override matched. Routing token to OAuth2 introspection.");
            return true;
        }
    }
    false
}

mod tests;

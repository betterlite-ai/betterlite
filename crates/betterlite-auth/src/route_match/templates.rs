use onig::{MatchParam, Regex, RegexOptions, SearchOptions, Syntax};

pub enum RouteValue<'text> {
    Text(&'text str),
    NonString,
}

#[must_use]
pub fn route_matches_pattern(route: &RouteValue<'_>, pattern: &str) -> bool {
    let RouteValue::Text(text) = route else {
        return false;
    };
    let substituted = substitute_placeholders(pattern);
    let Ok(compiled) = Regex::with_options(
        &format!("^{substituted}$"),
        RegexOptions::REGEX_OPTION_NONE,
        Syntax::python(),
    ) else {
        return false;
    };
    let mut limits = MatchParam::default();
    limits.set_retry_limit_in_match(1_000_000);
    compiled
        .match_with_param(*text, 0, SearchOptions::SEARCH_OPTION_NONE, None, limits)
        .is_ok_and(|matched| matched.is_some())
}

// A :path placeholder stops before a following literal colon, but otherwise accepts router path input.
fn substitute_placeholders(pattern: &str) -> String {
    let mut result = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        let (before, tail) = rest.split_at(start);
        result.push_str(before);
        let Some(after_open) = tail.strip_prefix('{') else {
            result.push_str(tail);
            return result;
        };
        let Some(close) = after_open.find('}') else {
            result.push_str(tail);
            return result;
        };
        let (placeholder, after_close) = after_open.split_at(close);
        let after = after_close.strip_prefix('}').unwrap_or(after_close);
        result.push_str(placeholder_fragment(placeholder, after));
        rest = after;
    }
    result.push_str(rest);
    result
}

fn placeholder_fragment(placeholder: &str, after: &str) -> &'static str {
    if placeholder.is_empty() {
        return "{}";
    }
    if !placeholder.ends_with(":path") {
        return "[^/]+";
    }
    if after.contains(':') {
        "[^:]+"
    } else {
        r"[\s\S]+"
    }
}

#[must_use]
pub fn route_matches_wildcard_pattern(route: &str, pattern: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or_else(|| route == pattern, |prefix| route.starts_with(prefix))
}

#[must_use]
pub fn route_matches_allowed_route(route: &str, allowed_route: &str) -> bool {
    if route == allowed_route {
        return true;
    }
    route.starts_with(&format!("{allowed_route}/"))
}

#[must_use]
pub fn check_route_access(route: &str, allowed_routes: &[&str]) -> bool {
    if allowed_routes.contains(&route) {
        return true;
    }
    let wildcard_hit = allowed_routes.iter().any(|pattern| {
        matches!(pattern.chars().last(), Some('*'))
            && route_matches_wildcard_pattern(route, pattern)
    });
    if wildcard_hit {
        return true;
    }
    allowed_routes
        .iter()
        .any(|pattern| route_matches_pattern(&RouteValue::Text(route), pattern))
}

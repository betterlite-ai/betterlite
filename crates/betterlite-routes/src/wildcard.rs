mod tests;

#[must_use]
pub fn route_matches_wildcard_pattern(route: &str, pattern: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or_else(|| route == pattern, |prefix| route.starts_with(prefix))
}

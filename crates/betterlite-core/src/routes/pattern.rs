use super::RouteMetadataError;

pub fn parse_route(route: &str) -> Result<Vec<&str>, RouteMetadataError> {
    if route.len() > 4096 {
        return Err(RouteMetadataError::RouteTooLong);
    }
    let trimmed = route.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let segments = trimmed.split('/').collect::<Vec<_>>();
    if segments.len() > 256 {
        return Err(RouteMetadataError::TooManySegments);
    }
    Ok(segments)
}

fn placeholder_suffix(pattern: &str) -> Option<&str> {
    if !pattern.starts_with('{') {
        return None;
    }
    let end = pattern.find('}')?;
    pattern.get(end..)?.strip_prefix('}')
}

fn segment_matches(route: &str, pattern: &str) -> bool {
    let Some(suffix) = placeholder_suffix(pattern) else {
        return route == pattern;
    };
    if suffix.is_empty() {
        return true;
    }
    route.ends_with(suffix) && route.len() > suffix.len()
}

pub(super) fn pattern_matches(route: &str, pattern: &str) -> Result<bool, RouteMetadataError> {
    let route_parts = parse_route(route)?;
    let pattern_parts = parse_route(pattern)?;
    if pattern_parts.is_empty() || route_parts.len() < pattern_parts.len() {
        return Ok(false);
    }
    let Some(tail) = pattern_parts.last() else {
        return Ok(false);
    };
    let spans = placeholder_suffix(tail).is_some_and(|suffix| !suffix.is_empty());
    if route_parts.len() > pattern_parts.len() && !spans {
        return Ok(false);
    }
    let Some(head_count) = pattern_parts.len().checked_sub(1) else {
        return Ok(false);
    };
    for (route_part, pattern_part) in route_parts
        .iter()
        .take(head_count)
        .zip(pattern_parts.iter())
    {
        if !segment_matches(route_part, pattern_part) {
            return Ok(false);
        }
    }
    let tail_value = route_parts
        .iter()
        .skip(head_count)
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    Ok(segment_matches(&tail_value, tail))
}

mod tests;

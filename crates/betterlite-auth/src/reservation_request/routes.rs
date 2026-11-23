const UNBILLED_ROUTES: [&str; 6] = [
    "/models",
    "/v1/models",
    "/utils/token_counter",
    "/responses/input_tokens",
    "/v1/responses/input_tokens",
    "/openai/v1/responses/input_tokens",
];

// _TOKEN_COUNTING_SEGMENTS: the resource a token counting route names before its colon
const TOKEN_COUNTING_SEGMENTS: [&str; 2] = ["count_tokens", "count-tokens"];

// _TOKEN_COUNTING_ACTION: the Google action a token counting route names after its colon
const TOKEN_COUNTING_ACTION: &str = "countTokens";

// _is_token_counting_route: the last path segment split at its first colon
#[must_use]
pub fn is_token_counting_route(route: &str) -> bool {
    let last_segment = route.rsplit_once('/').map_or(route, |(_, segment)| segment);
    let (resource, action) = last_segment.split_once(':').unwrap_or((last_segment, ""));
    TOKEN_COUNTING_SEGMENTS.contains(&resource) || action == TOKEN_COUNTING_ACTION
}

// _is_unbilled_route: one of the six exact routes or a token counting one, read off the raw path
#[must_use]
pub fn is_unbilled_route(route: &str) -> bool {
    if UNBILLED_ROUTES.contains(&route) {
        return true;
    }
    is_token_counting_route(route)
}

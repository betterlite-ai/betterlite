use super::CLAUDE_PLATFORM_BEDROCK_ROUTE;

#[must_use]
pub fn strip_claude_platform_route(model: &str) -> String {
    let Some(stripped) = model.strip_prefix(CLAUDE_PLATFORM_BEDROCK_ROUTE) else {
        return model.to_owned();
    };
    stripped.to_owned()
}

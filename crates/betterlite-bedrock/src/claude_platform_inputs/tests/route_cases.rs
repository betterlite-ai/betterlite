use crate::claude_platform_inputs::CLAUDE_PLATFORM_BEDROCK_ROUTE;

pub(super) fn platform_route_prefix() {
    let strip = |model: &str| {
        model
            .strip_prefix(CLAUDE_PLATFORM_BEDROCK_ROUTE)
            .unwrap_or(model)
            .to_owned()
    };
    assert_eq!(
        strip("claude_platform/anthropic.claude-3"),
        "anthropic.claude-3"
    );
    assert_eq!(
        strip("claude_platform/claude_platform/anthropic"),
        "claude_platform/anthropic"
    );
    assert_eq!(strip("claude_platform/"), "");
    assert_eq!(strip("claude_platform"), "claude_platform");
    assert_eq!(strip(""), "");
    assert_eq!(
        strip("xclaude_platform/anthropic"),
        "xclaude_platform/anthropic"
    );
    assert_eq!(
        strip("converse/claude_platform/anthropic"),
        "converse/claude_platform/anthropic"
    );
    assert_eq!(
        strip("anthropic.claude/claude_platform"),
        "anthropic.claude/claude_platform"
    );
}

use crate::claude_platform_inputs::route::strip_claude_platform_route;

pub(super) fn platform_route_prefix() {
    assert_eq!(
        strip_claude_platform_route("claude_platform/anthropic.claude-3"),
        "anthropic.claude-3"
    );
    assert_eq!(
        strip_claude_platform_route("claude_platform/claude_platform/anthropic"),
        "claude_platform/anthropic"
    );
    assert_eq!(strip_claude_platform_route("claude_platform/"), "");
    assert_eq!(
        strip_claude_platform_route("claude_platform"),
        "claude_platform"
    );
    assert_eq!(strip_claude_platform_route(""), "");
    assert_eq!(
        strip_claude_platform_route("xclaude_platform/anthropic"),
        "xclaude_platform/anthropic"
    );
    assert_eq!(
        strip_claude_platform_route("converse/claude_platform/anthropic"),
        "converse/claude_platform/anthropic"
    );
    assert_eq!(
        strip_claude_platform_route("anthropic.claude/claude_platform"),
        "anthropic.claude/claude_platform"
    );
}

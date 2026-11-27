from enum import Enum
from typing import Final

ANTHROPIC_API_HEADERS: Final = {"anthropic-version", "anthropic-beta"}

ANTHROPIC_API_ONLY_HEADERS: Final = {"anthropic-beta"}


class AnthropicHostedToolNames(str, Enum):
    WEB_SEARCH = "web_search"
    BASH = "bash"
    TEXT_EDITOR = "text_editor"
    CODE_EXECUTION = "code_execution"
    WEB_FETCH = "web_fetch"
    MEMORY = "memory"
    TOOL_SEARCH_TOOL = "tool_search_tool"


ANTHROPIC_HOSTED_TOOLS = AnthropicHostedToolNames


class AnthropicBetaHeaderValues(str, Enum):
    WEB_FETCH_2025_09_10 = "web-fetch-2025-09-10"
    WEB_SEARCH_2025_03_05 = "web-search-2025-03-05"
    CONTEXT_MANAGEMENT_2025_06_27 = "context-management-2025-06-27"
    COMPACT_2026_01_12 = "compact-2026-01-12"
    COMPACT_2026_09_04 = "compact-2026-09-04"
    STRUCTURED_OUTPUT_2025_09_25 = "structured-outputs-2025-11-13"
    ADVANCED_TOOL_USE_2025_11_20 = "advanced-tool-use-2025-11-20"
    FAST_MODE_2026_02_01 = "fast-mode-2026-02-01"
    ADVISOR_TOOL_2026_03_01 = "advisor-tool-2026-03-01"
    PER_TURN_CONTROL_2026_07_01 = "per-turn-control-2026-07-01"
    DANGEROUS_TOOL_USE_2026_09_03 = "dangerous-tool-use-2026-09-03"


ANTHROPIC_BETA_HEADER_VALUES = AnthropicBetaHeaderValues

# ast-grep-ignore: no-long-names
ANTHROPIC_TOOL_SEARCH_BETA_HEADER: Final = "advanced-tool-use-2025-11-20"

# ast-grep-ignore: no-long-names
ANTHROPIC_TOOL_SEARCH_TOOL_TYPES: Final = frozenset(
    {"tool_search_tool_regex_20251119", "tool_search_tool_bm25_20251119"}
)

ANTHROPIC_EFFORT_BETA_HEADER: Final = "effort-2025-11-24"

# ast-grep-ignore: no-long-names
ANTHROPIC_MID_CONVERSATION_OUTPUT_CONFIG_BETA_HEADER: Final = (
    "mid-conversation-output-config-2026-07-01"
)

# ast-grep-ignore: no-long-names
ANTHROPIC_THINKING_DISPLAY_UPDATES_BETA_HEADER: Final = "thinking-display-updates-2026-08-18"

# ast-grep-ignore: no-long-names
ANTHROPIC_MID_CONVERSATION_TOOL_CHANGES_BETA_HEADER: Final = (
    "mid-conversation-tool-changes-2026-07-01"
)

# ast-grep-ignore: no-long-names
ANTHROPIC_FINE_GRAINED_TOOL_STREAMING_BETA_HEADER: Final = "fine-grained-tool-streaming-2025-05-14"

# split in two, since ruff S105 reads a literal assigned to a token name as a password
ANTHROPIC_OAUTH_TOKEN_PREFIX: Final = "sk-ant-" + "oat"

ANTHROPIC_OAUTH_BETA_HEADER: Final = "oauth-2025-04-20"

# ast-grep-ignore: no-long-names
ANTHROPIC_PROMPT_CACHING_SCOPE_BETA_HEADER: Final = "prompt-caching-scope-2026-01-05"

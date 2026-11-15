from collections.abc import Mapping
from types import MappingProxyType
from typing import Final

# ast-grep-ignore: no-long-names
TOOL_SEARCH_BETA_HEADER_ANTHROPIC: Final = "advanced-tool-use-2025-11-20"

TOOL_SEARCH_BETA_HEADER_VERTEX: Final = "tool-search-tool-2025-10-19"

# ast-grep-ignore: no-long-names
TOOL_SEARCH_BETA_HEADER_BEDROCK: Final = "tool-search-tool-2025-10-19"

# ast-grep-ignore: no-long-names
TOOL_SEARCH_BETA_HEADER_BY_PROVIDER: Final[Mapping[str, str]] = MappingProxyType(
    {
        "anthropic": TOOL_SEARCH_BETA_HEADER_ANTHROPIC,  # ast-grep-ignore: no-long-names
        "azure": TOOL_SEARCH_BETA_HEADER_ANTHROPIC,  # ast-grep-ignore: no-long-names
        "azure_ai": TOOL_SEARCH_BETA_HEADER_ANTHROPIC,  # ast-grep-ignore: no-long-names
        "vertex_ai": TOOL_SEARCH_BETA_HEADER_VERTEX,
        "vertex_ai_beta": TOOL_SEARCH_BETA_HEADER_VERTEX,
        "bedrock": TOOL_SEARCH_BETA_HEADER_BEDROCK,  # ast-grep-ignore: no-long-names
    }
)


def get_tool_search_beta_header(custom_llm_provider: str) -> str:
    # ast-grep-ignore: no-long-names
    header: str | None = TOOL_SEARCH_BETA_HEADER_BY_PROVIDER.get(custom_llm_provider)
    if header is None:
        # ast-grep-ignore: no-long-names
        return TOOL_SEARCH_BETA_HEADER_ANTHROPIC
    return header

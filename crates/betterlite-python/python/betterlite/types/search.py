from pydantic import JsonValue
from typing_extensions import Required, TypedDict


class SearchToolLiteLLMParams(TypedDict, total=False):
    search_provider: Required[str]
    api_key: str | None
    api_base: str | None
    timeout: float | None
    max_retries: int | None


class SearchTool(TypedDict, total=False):
    search_tool_id: str | None
    search_tool_name: Required[str]
    litellm_params: Required[SearchToolLiteLLMParams]
    search_tool_info: dict[str, JsonValue] | None
    created_at: str | None
    updated_at: str | None


class SearchToolInfoResponse(TypedDict, total=False):
    search_tool_id: str | None
    search_tool_name: str
    litellm_params: dict[str, JsonValue]
    search_tool_info: dict[str, JsonValue] | None
    created_at: str | None
    updated_at: str | None
    is_from_config: bool | None


class ListSearchToolsResponse(TypedDict):
    search_tools: list[SearchToolInfoResponse]


class AvailableSearchProvider(TypedDict):
    provider_name: str
    ui_friendly_name: str

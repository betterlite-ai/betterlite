from dataclasses import dataclass
from typing import Literal, TypeAlias, TypedDict

from pydantic import BaseModel
from typing_extensions import ReadOnly


class AnthropicSearchQuery(BaseModel):
    query: str


class AnthropicServerToolUseBlock(BaseModel):
    type: Literal["server_tool_use"] = "server_tool_use"
    id: str
    name: Literal["web_search"] = "web_search"
    input: AnthropicSearchQuery


class RichWebSearchInput(TypedDict, total=False):
    objective: ReadOnly[str]
    search_queries: ReadOnly[list[str]]


WebSearchToolResultErrorCode: TypeAlias = Literal[
    "invalid_tool_input",
    "unavailable",
    "max_uses_exceeded",
    "too_many_requests",
    "query_too_long",
    "request_too_large",
]


@dataclass(frozen=True, slots=True)
class SearchSucceeded:
    text: str
    response: BaseModel | None


@dataclass(frozen=True, slots=True)
class SearchFailed:
    error_code: WebSearchToolResultErrorCode
    message: str


SearchOutcome: TypeAlias = SearchSucceeded | SearchFailed


class WebSearchInterceptionConfig(TypedDict, total=False):
    enabled_providers: list[str]
    search_tool_name: str | None
    max_agentic_loops: ReadOnly[int | None]

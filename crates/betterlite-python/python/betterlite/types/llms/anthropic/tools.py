from typing import Final, Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.openai.core.chat_blocks import ChatCompletionCachedContent


class AnthropicMessagesToolChoice(TypedDict, total=False):
    type: Required[Literal["auto", "any", "tool", "none"]]
    name: str
    disable_parallel_tool_use: bool


AnthropicInputSchema = TypedDict(
    "AnthropicInputSchema",
    {
        "type": str | None,
        "properties": dict[str, JsonValue] | None,
        "additionalProperties": bool | None,
        "required": list[str] | None,
        "$defs": dict[str, JsonValue] | None,
        "strict": bool | None,
    },
    total=False,
)


class AnthropicOutputSchema(TypedDict, total=False):
    type: Required[Literal["json_schema"]]
    schema: Required[dict[str, JsonValue]]
    strict: ReadOnly[bool]


class AnthropicOutputConfig(TypedDict, total=False):
    effort: Literal["high", "medium", "low", "xhigh", "max"]
    format: AnthropicOutputSchema


class AnthropicMessagesTool(TypedDict, total=False):
    name: Required[str]
    description: str
    input_schema: AnthropicInputSchema | None
    strict: ReadOnly[bool]
    type: Literal["custom"]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    defer_loading: bool
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None
    eager_input_streaming: ReadOnly[bool]


class AnthropicComputerTool(TypedDict, total=False):
    display_width_px: Required[int]
    display_height_px: Required[int]
    display_number: int
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    type: Required[str]
    name: Required[str]


class AnthropicWebSearchUserLocation(TypedDict, total=False):
    city: str | None
    country: str | None
    region: str | None
    timezone: str | None
    type: Required[Literal["approximate"]]


class AnthropicWebSearchTool(TypedDict, total=False):
    name: Required[Literal["web_search"]]
    type: Required[str]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    max_uses: int | None
    user_location: AnthropicWebSearchUserLocation | None
    defer_loading: bool | None
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None


class AnthropicHostedTools(TypedDict, total=False):
    type: Required[str]
    name: Required[str]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    defer_loading: bool | None
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None


class AnthropicCodeExecutionTool(TypedDict, total=False):
    type: Required[str]
    name: Required[Literal["code_execution"]]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    defer_loading: bool | None
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None


class AnthropicMemoryTool(TypedDict, total=False):
    type: Required[str]
    name: Required[Literal["memory"]]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    defer_loading: bool | None
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None


class AnthropicToolSearchToolRegex(TypedDict, total=False):
    type: Required[Literal["tool_search_tool_regex_20251119"]]
    name: Required[str]


class AnthropicToolSearchToolBM25(TypedDict, total=False):
    type: Required[Literal["tool_search_tool_bm25_20251119"]]
    name: Required[str]
    cache_control: dict[str, JsonValue] | ChatCompletionCachedContent | None
    defer_loading: bool | None
    allowed_callers: list[str] | None
    input_examples: list[dict[str, JsonValue]] | None


ANTHROPIC_ADVISOR_TOOL_TYPE: Final = "advisor_20260301"


class AnthropicAdvisorTool(TypedDict, total=False):
    type: Required[Literal["advisor_20260301"]]
    name: Required[Literal["advisor"]]
    model: Required[str]
    max_uses: int | None
    caching: dict[str, JsonValue] | None


class ToolReference(TypedDict, total=False):
    type: Required[Literal["tool_reference"]]
    tool_name: Required[str]


class DirectToolCaller(TypedDict, total=False):
    type: Required[Literal["direct"]]


class CodeExecutionToolCaller(TypedDict, total=False):
    type: Required[Literal["code_execution_20250825"]]
    tool_id: Required[str]


ToolCaller: TypeAlias = DirectToolCaller | CodeExecutionToolCaller


class AnthropicContainer(TypedDict, total=False):
    id: Required[str]
    expires_at: str | None


AllAnthropicToolsValues: TypeAlias = (
    AnthropicComputerTool
    | AnthropicHostedTools
    | AnthropicMessagesTool
    | AnthropicWebSearchTool
    | AnthropicCodeExecutionTool
    | AnthropicMemoryTool
    | AnthropicToolSearchToolRegex
    | AnthropicToolSearchToolBM25
    | AnthropicAdvisorTool
)


# ast-grep-ignore: no-long-names
class AnthropicMcpServerToolConfiguration(TypedDict, total=False):
    allowed_tools: list[str] | None


class AnthropicMcpServerTool(TypedDict, total=False):
    type: Required[Literal["url"]]
    url: Required[str]
    name: Required[str]
    # ast-grep-ignore: no-long-names
    tool_configuration: AnthropicMcpServerToolConfiguration
    authorization_token: str

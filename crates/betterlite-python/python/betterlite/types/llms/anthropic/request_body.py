from typing import Literal

from pydantic import JsonValue
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.anthropic.messages import (
    AllAnthropicMessageValues,
    AnthropicCompaction,
    AnthropicMetadata,
)
from betterlite.types.llms.anthropic.tools import (
    AllAnthropicToolsValues,
    AnthropicMcpServerTool,
    AnthropicMessagesToolChoice,
    AnthropicOutputConfig,
    AnthropicOutputSchema,
)


# ast-grep-ignore: no-long-names
class AnthropicMessagesRequestOptionalParams(TypedDict, total=False):
    max_tokens: int | None
    metadata: AnthropicMetadata | dict[str, JsonValue] | None
    stop_sequences: list[str] | None
    stream: bool | None
    system: str | list[JsonValue] | None
    temperature: float | None
    thinking: dict[str, JsonValue] | None
    tool_choice: AnthropicMessagesToolChoice | dict[str, JsonValue] | None
    tools: list[AllAnthropicToolsValues | dict[str, JsonValue]] | None
    top_k: int | None
    inference_geo: str | None
    top_p: float | None
    mcp_servers: list[AnthropicMcpServerTool] | None
    context_management: dict[str, JsonValue] | None
    compaction: ReadOnly[AnthropicCompaction | None]
    container: dict[str, JsonValue] | None
    output_format: AnthropicOutputSchema | None
    speed: str | None
    output_config: AnthropicOutputConfig | None
    cache_control: dict[str, JsonValue] | None
    reasoning_effort: str | None
    safeguards: ReadOnly[list[dict[str, JsonValue]] | None]


# ast-grep-ignore: no-long-names
class AnthropicMessagesRequest(AnthropicMessagesRequestOptionalParams, total=False):
    model: Required[str]
    messages: Required[list[AllAnthropicMessageValues] | list[dict[str, JsonValue]]]
    litellm_metadata: dict[str, JsonValue]


class AnthropicThinkingParam(TypedDict, total=False):
    type: ReadOnly[Literal["enabled", "adaptive", "disabled"]]
    budget_tokens: int
    display: ReadOnly[Literal["summarized", "omitted", "updates"]]

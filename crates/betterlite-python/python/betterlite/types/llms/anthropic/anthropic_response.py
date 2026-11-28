from collections.abc import Sequence
from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import NotRequired, ReadOnly, TypedDict

from betterlite.types.llms.anthropic.messages import CompactionBlock
from betterlite.types.llms.anthropic.responses import (
    AnthropicResponseContentBlockText,
    AnthropicResponseContentBlockToolUse,
)
from betterlite.types.llms.anthropic.streaming import (
    AnthropicStopDetails,
    ContextManagementResponse,
    ServerToolUsage,
    UsageIteration,
)


class AnthropicResponseTextBlock(TypedDict, total=False):
    citations: list[dict[str, JsonValue]] | None
    text: str
    type: Literal["text"]


class AnthropicResponseToolUseBlock(TypedDict, total=False):
    id: str | None
    input: str | None
    name: str | None
    type: Literal["tool_use"]


class AnthropicResponseThinkingBlock(TypedDict, total=False):
    signature: str | None
    thinking: str | None
    type: Literal["thinking"]


# ast-grep-ignore: no-long-names
class AnthropicResponseRedactedThinkingBlock(TypedDict, total=False):
    data: str | None
    type: Literal["redacted_thinking"]


AnthropicResponseContentBlock: TypeAlias = (
    AnthropicResponseTextBlock
    | AnthropicResponseToolUseBlock
    | AnthropicResponseThinkingBlock
    # ast-grep-ignore: no-long-names
    | AnthropicResponseRedactedThinkingBlock
    | CompactionBlock
)


class AnthropicUsage(TypedDict, total=False):
    input_tokens: int
    output_tokens: int
    iterations: ReadOnly[Sequence[UsageIteration]]
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
    server_tool_use: NotRequired[ReadOnly[ServerToolUsage]]


class AnthropicMessagesResponse(TypedDict, total=False):
    content: (
        list[
            AnthropicResponseContentBlock
            # ast-grep-ignore: no-long-names
            | AnthropicResponseContentBlockText
            # ast-grep-ignore: no-long-names
            | AnthropicResponseContentBlockToolUse
        ]
        | None
    )
    id: str
    model: str | None
    role: Literal["assistant"] | None
    stop_reason: ReadOnly[
        Literal["end_turn", "max_tokens", "stop_sequence", "tool_use", "refusal", "compaction"]
        | None
    ]
    stop_details: NotRequired[ReadOnly[AnthropicStopDetails | None]]
    stop_sequence: str | None
    type: Literal["message"] | None
    usage: AnthropicUsage | None
    context_management: NotRequired[ContextManagementResponse]
    safeguard_results: NotRequired[ReadOnly[list[dict[str, JsonValue]]]]

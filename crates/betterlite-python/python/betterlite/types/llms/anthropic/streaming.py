from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import NotRequired, ReadOnly, Required, TypedDict

from betterlite.types.llms.anthropic.tools import ToolCaller
from betterlite.types.llms.openai.core.chat_blocks import ChatCompletionThinkingBlock


class ContentTextBlockDelta(TypedDict):
    type: str
    text: str


class ContentCitationsBlockDelta(TypedDict):
    type: Literal["citations"]
    citation: dict[str, JsonValue]


class ContentJsonBlockDelta(TypedDict):
    type: str
    partial_json: str


class ContentThinkingBlockDelta(TypedDict):
    type: Literal["thinking_delta"]
    thinking: str


# ast-grep-ignore: no-long-names
class ContentThinkingSignatureBlockDelta(TypedDict):
    type: Literal["signature_delta"]
    signature: str


StreamingContentBlockDeltaType: TypeAlias = Literal[
    "text_delta", "input_json_delta", "thinking_delta", "signature_delta"
]


class ContentBlockDelta(TypedDict):
    type: Literal["content_block_delta"]
    index: int
    delta: (
        ContentTextBlockDelta
        | ContentJsonBlockDelta
        | ContentCitationsBlockDelta
        | ContentThinkingBlockDelta
        # ast-grep-ignore: no-long-names
        | ContentThinkingSignatureBlockDelta
    )


class ContentBlockStop(TypedDict):
    type: Literal["content_block_stop"]
    index: int


class ToolUseBlock(TypedDict):
    id: str
    input: dict[str, JsonValue]
    name: str
    type: Literal["tool_use"]
    caller: ToolCaller | None


class TextBlock(TypedDict):
    text: str
    type: Literal["text"]


class ContentBlockStartToolUse(TypedDict):
    type: Literal["content_block_start"]
    id: str
    name: str
    input: dict[str, JsonValue]
    content_block: ToolUseBlock


class ContentBlockStartText(TypedDict):
    type: Literal["content_block_start"]
    index: int
    content_block: TextBlock


ContentBlockContentBlockDict: TypeAlias = ToolUseBlock | TextBlock | ChatCompletionThinkingBlock

ContentBlockStart: TypeAlias = ContentBlockStartToolUse | ContentBlockStartText


class AnthropicStopDetails(TypedDict, total=False):
    type: ReadOnly[Literal["refusal"]]
    category: ReadOnly[str | None]
    explanation: ReadOnly[str | None]


class MessageDelta(TypedDict, total=False):
    stop_reason: str | None
    stop_details: ReadOnly[AnthropicStopDetails]
    safeguard_results: ReadOnly[list[dict[str, JsonValue]]]


class ServerToolUsage(TypedDict, total=False):
    web_search_requests: ReadOnly[int]


class UsageDelta(TypedDict, total=False):
    input_tokens: int
    output_tokens: int
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
    server_tool_use: ReadOnly[ServerToolUsage]


class AppliedEdit(TypedDict, total=False):
    type: str
    cleared_input_tokens: int
    cleared_tool_uses: int
    cleared_thinking_turns: int
    summary_input_tokens: int
    summary_output_tokens: int
    error: str
    warnings: list[str]


class ContextManagementResponse(TypedDict, total=False):
    applied_edits: list[AppliedEdit]


class UsageIteration(TypedDict, total=False):
    type: Required[Literal["compaction", "message"]]
    input_tokens: int
    output_tokens: int


class MessageBlockDelta(TypedDict):
    type: Literal["message_delta"]
    delta: MessageDelta
    usage: NotRequired[ReadOnly[UsageDelta]]
    context_management: NotRequired[ContextManagementResponse]


class MessageChunk(TypedDict, total=False):
    id: str
    type: str
    role: str
    model: str
    content: list[JsonValue]
    stop_reason: str | None
    stop_sequence: str | None
    usage: UsageDelta
    safeguard_results: ReadOnly[list[dict[str, JsonValue]]]


class MessageStartBlock(TypedDict):
    type: Literal["message_start"]
    message: MessageChunk

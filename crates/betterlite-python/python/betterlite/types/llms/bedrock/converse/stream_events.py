from typing import Required

from typing_extensions import TypedDict

from betterlite.types.llms.bedrock.converse.citations import CitationsContentBlock
from betterlite.types.llms.bedrock.converse.content import (
    BedrockConverseReasoningContentBlockDelta,
)
from betterlite.types.llms.bedrock.converse.messages import ConverseTokenUsageBlock
from betterlite.types.llms.bedrock.converse.tool_call_chunk import ChatCompletionToolCallChunk


class ToolBlockDeltaEvent(TypedDict):
    input: str


class ToolUseBlockStartEvent(TypedDict):
    name: str
    toolUseId: str


class ContentBlockStartEvent(TypedDict, total=False):
    toolUse: ToolUseBlockStartEvent | None
    reasoningContent: BedrockConverseReasoningContentBlockDelta  # ast-grep-ignore: no-long-names


class ContentBlockDeltaEvent(TypedDict, total=False):
    text: str
    toolUse: ToolBlockDeltaEvent
    reasoningContent: BedrockConverseReasoningContentBlockDelta  # ast-grep-ignore: no-long-names
    citationsContent: CitationsContentBlock


class GenericStreamingChunk(TypedDict):
    text: Required[str]
    tool_use: ChatCompletionToolCallChunk | None
    is_finished: Required[bool]
    finish_reason: Required[str]
    usage: ConverseTokenUsageBlock | None
    index: int


class Document(TypedDict):
    title: str
    snippet: str

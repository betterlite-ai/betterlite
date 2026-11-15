from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.llms.bedrock.converse.citations import CitationsContentBlock
from betterlite.types.llms.bedrock.converse.content import (
    BedrockConverseReasoningContentBlock,
    CachePointBlock,
    DocumentBlock,
    GuardrailConverseContentBlock,
    ImageBlock,
    ToolResultBlock,
    ToolUseBlock,
    VideoBlock,
)


class ContentBlock(TypedDict, total=False):
    text: str
    image: ImageBlock
    video: VideoBlock
    document: DocumentBlock
    toolResult: ToolResultBlock
    toolUse: ToolUseBlock
    cachePoint: CachePointBlock
    reasoningContent: BedrockConverseReasoningContentBlock  # ast-grep-ignore: no-long-names
    guardContent: GuardrailConverseContentBlock
    citationsContent: CitationsContentBlock


class MessageBlock(TypedDict):
    content: list[ContentBlock]
    role: Literal["user", "assistant"]


class ConverseMetricsBlock(TypedDict):
    latencyMs: float


class ConverseResponseOutputBlock(TypedDict):
    message: MessageBlock | None


class CacheDetailBlock(TypedDict):
    inputTokens: ReadOnly[int]
    ttl: ReadOnly[Literal["5m", "1h"]]


class ConverseTokenUsageBlock(TypedDict, total=False):
    inputTokens: Required[ReadOnly[int]]
    outputTokens: Required[ReadOnly[int]]
    totalTokens: ReadOnly[int]
    cacheReadInputTokenCount: ReadOnly[int]
    cacheReadInputTokens: ReadOnly[int]
    cacheWriteInputTokenCount: ReadOnly[int]
    cacheWriteInputTokens: ReadOnly[int]
    cacheDetails: ReadOnly[list[CacheDetailBlock]]


class ServiceTierBlock(TypedDict):
    type: Literal["priority", "default", "flex"]


class ConverseResponseBlock(TypedDict, total=False):
    additionalModelResponseFields: dict[str, JsonValue]
    metrics: ConverseMetricsBlock
    output: Required[ConverseResponseOutputBlock]
    stopReason: Required[str]
    usage: Required[ConverseTokenUsageBlock]
    serviceTier: ServiceTierBlock

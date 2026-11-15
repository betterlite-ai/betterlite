from typing import Literal, Required, TypeAlias

from pydantic import BaseModel, JsonValue
from typing_extensions import ReadOnly, TypedDict


class ChatCompletionUsageBlock(TypedDict, total=False):
    prompt_tokens: Required[int]
    completion_tokens: Required[int]
    total_tokens: Required[int]
    prompt_tokens_details: dict[str, JsonValue] | None
    completion_tokens_details: dict[str, JsonValue] | None


class GenericStreamingChunk(TypedDict, total=False):
    text: Required[str]
    is_finished: Required[bool]
    finish_reason: Required[str | None]
    logprobs: BaseModel | None
    original_chunk: BaseModel | None
    usage: BaseModel | None


class DatabricksTextContent(TypedDict, total=False):
    type: Literal["text"]
    text: Required[str]
    citations: list[dict[str, JsonValue]] | None


class DatabricksReasoningSummary(TypedDict):
    type: Literal["summary_text"]
    text: str
    signature: str


class DatabricksReasoningContent(TypedDict, total=False):
    type: Literal["reasoning"]
    summary: Required[list[DatabricksReasoningSummary]]
    citations: list[dict[str, JsonValue]] | None


AllDatabricksContentListValues: TypeAlias = DatabricksTextContent | DatabricksReasoningContent

AllDatabricksContentValues: TypeAlias = str | list[AllDatabricksContentListValues]


class DatabricksFunction(TypedDict, total=False):
    name: Required[str]
    description: dict[str, JsonValue] | str
    parameters: dict[str, JsonValue]
    strict: bool


class DatabricksTool(TypedDict):
    function: DatabricksFunction
    type: Literal["function"]


class DatabricksMessage(TypedDict, total=False):
    role: Required[str]
    content: Required[AllDatabricksContentValues]
    tool_calls: list[DatabricksTool] | None
    reasoning_content: ReadOnly[str | None]
    reasoning: ReadOnly[str | None]


class DatabricksDelta(TypedDict, total=False):
    role: ReadOnly[str]
    content: ReadOnly[AllDatabricksContentValues | None]
    reasoning_content: ReadOnly[str | None]


class DatabricksChoice(TypedDict, total=False):
    index: Required[int]
    message: Required[DatabricksMessage]
    finish_reason: Required[str | None]
    extra_fields: str


class DatabricksResponse(TypedDict):
    id: str
    object: str
    created: int
    model: str
    choices: list[DatabricksChoice]
    usage: ChatCompletionUsageBlock

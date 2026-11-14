from typing import Literal, Required, TypeAlias

from pydantic import JsonValue
from typing_extensions import TypedDict


class OpenAIChatCompletionLogprobs(TypedDict, total=False):
    content: list[dict[str, JsonValue]]
    refusal: list[dict[str, JsonValue]]


class OpenAIChatCompletionChoices(TypedDict, total=False):
    finish_reason: Required[str]
    index: Required[int]
    logprobs: OpenAIChatCompletionLogprobs | None
    message: Required[dict[str, JsonValue]]


class OpenAIChatCompletionResponse(TypedDict, total=False):
    id: Required[str]
    object: Required[str]
    created: Required[int]
    model: Required[str]
    choices: Required[list[OpenAIChatCompletionChoices]]
    usage: Required[dict[str, JsonValue]]
    system_fingerprint: str
    service_tier: str


class OpenAIBatchResponse(TypedDict, total=False):
    status_code: int
    request_id: str
    body: OpenAIChatCompletionResponse | dict[str, JsonValue]


class OpenAIBatchResult(TypedDict, total=False):
    custom_id: str
    response: OpenAIBatchResponse


# ast-grep-ignore: no-long-names
OpenAIChatCompletionFinishReason: TypeAlias = Literal[
    "stop",
    "content_filter",
    "function_call",
    "tool_calls",
    "length",
    "guardrail_intervened",
    "eos",
    "finish_reason_unspecified",
    "malformed_function_call",
]

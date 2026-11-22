from typing import Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import NotRequired, TypedDict


class WireMessage(TypedDict):
    role: str
    content: NotRequired[JsonValue]
    function_call: NotRequired[JsonValue]
    tool_calls: NotRequired[list[JsonValue] | None]
    audio: NotRequired[JsonValue]
    images: NotRequired[list[JsonValue] | None]
    annotations: NotRequired[list[JsonValue] | None]
    reasoning_content: NotRequired[JsonValue]
    thinking_blocks: NotRequired[list[JsonValue] | None]
    refusal: NotRequired[JsonValue]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class WireChoice(TypedDict):
    index: int
    message: WireMessage
    finish_reason: NotRequired[str | None]
    logprobs: NotRequired[JsonValue]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class WireUsage(TypedDict):
    prompt_tokens: NotRequired[int]
    completion_tokens: NotRequired[int]
    total_tokens: NotRequired[int]
    prompt_tokens_details: NotRequired[dict[str, JsonValue] | None]
    completion_tokens_details: NotRequired[dict[str, JsonValue] | None]
    cost: NotRequired[float | None]


class WireResponse(TypedDict):
    id: str
    created: int
    model: str
    object: str
    choices: list[WireChoice]
    usage: NotRequired[WireUsage]
    system_fingerprint: NotRequired[str | None]
    service_tier: NotRequired[str | None]
    moderation: NotRequired[JsonValue]


class WireDelta(TypedDict):
    content: NotRequired[JsonValue]
    role: NotRequired[str | None]
    function_call: NotRequired[JsonValue]
    tool_calls: NotRequired[list[JsonValue] | None]
    audio: NotRequired[JsonValue]
    images: NotRequired[list[JsonValue] | None]
    annotations: NotRequired[list[JsonValue] | None]
    reasoning_content: NotRequired[JsonValue]
    thinking_blocks: NotRequired[list[JsonValue] | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class WireStreamChoice(TypedDict):
    index: int
    delta: WireDelta
    finish_reason: NotRequired[str | None]
    logprobs: NotRequired[JsonValue]


class WireChunk(TypedDict):
    id: str
    created: int
    model: str
    object: str
    choices: NotRequired[list[WireStreamChoice]]
    usage: NotRequired[WireUsage]
    system_fingerprint: NotRequired[str | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]
    hidden_params: NotRequired[dict[str, JsonValue]]


class TokenCountPayload(TypedDict):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str
    original_response: NotRequired[dict[str, JsonValue] | None]
    error: NotRequired[bool]
    error_message: NotRequired[str | None]
    status_code: NotRequired[int | None]


RESPONSE: Final[TypeAdapter[WireResponse]] = TypeAdapter(WireResponse)
CHUNK: Final[TypeAdapter[WireChunk]] = TypeAdapter(WireChunk)
TOKEN_COUNT: Final[TypeAdapter[TokenCountPayload]] = TypeAdapter(TokenCountPayload)

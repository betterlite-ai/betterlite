from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import ClassVar, Final, Literal

from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import Required, Self, TypedDict, override

from betterlite.types.llms.openai.core.chat_blocks import ChatCompletionToolCallChunk
from betterlite.types.llms.openai.core.messages import AllMessageValues
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.tool_calls import ChatCompletionMessageToolCall


class GuardrailToolParam(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    type: str


class GenericGuardrailAPIMetadata(TypedDict, total=False):
    user_api_key_hash: str | None
    user_api_key_alias: str | None
    user_api_key_user_id: str | None
    user_api_key_user_email: str | None
    user_api_key_team_id: str | None
    user_api_key_team_alias: str | None
    user_api_key_end_user_id: str | None
    user_api_key_org_id: str | None


# ast-grep-ignore: no-long-names
class GenericGuardrailAPIOptionalParams(BaseModel):
    # ast-grep-ignore: no-long-names
    additional_provider_specific_params: dict[str, JsonValue] | None = None
    unreachable_fallback: Literal["fail_closed", "fail_open"] | None = "fail_closed"
    fail_on_error: bool | None = True
    streaming_end_of_stream_only: bool | None = None
    streaming_sampling_rate: int | None = Field(default=None, ge=1)
    streaming_transform_mode: Literal["block_only", "incremental_diff"] | None = None


class GenericGuardrailAPIConfigModel(
    # ast-grep-ignore: no-long-names
    GuardrailConfigModel[GenericGuardrailAPIOptionalParams],
):
    # ast-grep-ignore: no-long-names
    optional_params: GenericGuardrailAPIOptionalParams | None = Field(
        # ast-grep-ignore: no-long-names
        default_factory=GenericGuardrailAPIOptionalParams,
    )

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Generic Guardrail API"


class GenericGuardrailAPIRequest(BaseModel):
    input_type: Literal["request", "response"]
    litellm_call_id: str | None = None
    litellm_trace_id: str | None = None
    structured_messages: list[AllMessageValues] | None = None
    images: list[str] | None = None
    tools: list[GuardrailToolParam] | None = None
    texts: list[str] | None = None
    request_data: GenericGuardrailAPIMetadata
    request_headers: dict[str, str] | None = None
    litellm_version: str | None = None
    # ast-grep-ignore: no-long-names
    additional_provider_specific_params: dict[str, JsonValue] | None = None
    tool_calls: (
        list[ChatCompletionToolCallChunk] | list[ChatCompletionMessageToolCall] | None
    ) = None
    model: str | None = None


def coerce_stream_holdback_value(value: JsonValue) -> int:
    if value is None:
        return 0
    match value:
        case bool() | int() | float() | str():
            try:
                return max(0, int(value))
            except (TypeError, ValueError):
                return 0
        case list() | Mapping():
            return 0


class _StructuredMessage(TypedDict, total=False):
    role: Required[str]


_STRUCTURED_MESSAGES_ADAPTER: Final[TypeAdapter[list[_StructuredMessage]]] = TypeAdapter(
    list[_StructuredMessage],
    config=ConfigDict(extra="allow"),
)


# ast-grep-ignore: no-long-names
def structured_messages_from_response(value: JsonValue) -> Sequence[_StructuredMessage] | None:
    try:
        messages = _STRUCTURED_MESSAGES_ADAPTER.validate_python(value)
    except ValidationError:
        return None
    return messages


class _ResponsePayload(TypedDict, total=False):
    action: str | None
    blocked_reason: str | None
    texts: list[str] | None
    images: list[str] | None
    tools: list[GuardrailToolParam] | None
    stream_holdback_chars: JsonValue
    structured_messages: JsonValue


_RESPONSE_PAYLOAD_ADAPTER: Final[TypeAdapter[_ResponsePayload]] = TypeAdapter(_ResponsePayload)


@dataclass(eq=False, repr=False)
class GenericGuardrailAPIResponse:
    action: str
    texts: list[str] | None = None
    blocked_reason: str | None = None
    images: list[str] | None = None
    tools: list[GuardrailToolParam] | None = None
    stream_holdback_chars: list[int] | None = None
    structured_messages: Sequence[_StructuredMessage] | None = None

    @classmethod
    def from_dict(cls, data: dict[str, JsonValue]) -> Self:
        payload = _RESPONSE_PAYLOAD_ADAPTER.validate_python(data)
        raw_holdback = payload.get("stream_holdback_chars")
        stream_holdback_chars: list[int] | None = None
        match raw_holdback:
            case list():
                stream_holdback_chars = [
                    coerce_stream_holdback_value(value) for value in raw_holdback
                ]
            case dict() | bool() | int() | float() | str() | None:
                stream_holdback_chars = None
        action = payload.get("action")
        if action is None:
            action = "NONE"
        return cls(
            action=action,
            blocked_reason=payload.get("blocked_reason"),
            texts=payload.get("texts"),
            images=payload.get("images"),
            tools=payload.get("tools"),
            stream_holdback_chars=stream_holdback_chars,
            structured_messages=structured_messages_from_response(
                payload.get("structured_messages"),
            ),
        )

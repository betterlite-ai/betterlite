import logging
from collections.abc import Mapping
from typing import Final, Literal, TypeAlias

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.message import Delta, Message
from betterlite.types.utils.tool_calls import ChoiceLogprobs
from betterlite.types.utils.values import JsonValue, StoredValue, ValidationError

FinishReason: TypeAlias = Literal[
    "stop", "length", "tool_calls", "content_filter", "function_call",
]

FINISH_REASON_MAP: Final[dict[str, FinishReason]] = {
    "stop_sequence": "stop", "end_turn": "stop", "max_tokens": "length",
    "tool_use": "tool_calls", "refusal": "content_filter", "compaction": "length",
    "COMPLETE": "stop", "ERROR_TOXIC": "content_filter", "ERROR": "stop",
    "eos_token": "stop", "eos": "stop", "STOP": "stop",
    "MAX_TOKENS": "length", "SAFETY": "content_filter", "RECITATION": "content_filter",
    "FINISH_REASON_UNSPECIFIED": "stop", "MALFORMED_FUNCTION_CALL": "stop",
    "LANGUAGE": "content_filter", "OTHER": "content_filter", "BLOCKLIST": "content_filter",
    "PROHIBITED_CONTENT": "content_filter", "SPII": "content_filter",
    "IMAGE_SAFETY": "content_filter", "IMAGE_PROHIBITED_CONTENT": "content_filter",
    "TOO_MANY_TOOL_CALLS": "stop", "MALFORMED_RESPONSE": "stop", "NO_IMAGE": "content_filter",
    "IMAGE_RECITATION": "content_filter", "IMAGE_OTHER": "content_filter",
    "ESCALATION": "content_filter", "UNEXPECTED_TOOL_CALL": "stop",
    "MISSING_THOUGHT_SIGNATURE": "stop", "network_error": "stop",
    "sensitive": "content_filter", "guardrail_intervened": "content_filter",
    "stop": "stop", "length": "length", "tool_calls": "tool_calls",
    "function_call": "function_call", "content_filter": "content_filter",
    "content_filtered": "content_filter", "error": "stop",
}

LOGGER: Final = logging.getLogger("LiteLLM")


def map_finish_reason(finish_reason: str) -> FinishReason:
    mapped = FINISH_REASON_MAP.get(finish_reason)
    if mapped is None:
        LOGGER.warning("Unmapped finish_reason '%s', defaulting to 'stop'", finish_reason)
        return "stop"
    return mapped


class Choices(ResponseObject):
    finish_reason: FinishReason
    index: int
    message: Message
    logprobs: ChoiceLogprobs | JsonValue
    provider_specific_fields: dict[str, StoredValue] | None

    @override
    def __init__(
        self, finish_reason: str | None = None, index: int | None = None,
        message: StoredValue = None, logprobs: StoredValue = None,
        enhancements: StoredValue = None, provider_specific_fields: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        fields = provider_specific_fields
        mapped = "stop" if finish_reason is None else map_finish_reason(finish_reason)
        params["finish_reason"] = mapped
        if finish_reason is not None and finish_reason != mapped:
            fields = native_finish_reason(provider_specific_fields, finish_reason)
        params.update(index=0 if index is None else index, message=message_value(message))
        params["logprobs"] = logprobs_value(logprobs)
        super().__init__(**params)
        if enhancements is not None:
            self["enhancements"] = enhancements
        self.provider_specific_fields = provider_fields(fields)
        if self.logprobs is None:
            del self.logprobs
        if self.provider_specific_fields is None:
            del self.provider_specific_fields


class StreamingChoices(ResponseObject):
    finish_reason: FinishReason | None
    index: int
    delta: Delta
    logprobs: ChoiceLogprobs | JsonValue

    @override
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2007-L2038
        self, finish_reason: StoredValue = None, index: StoredValue = 0,
        delta: StoredValue = None, logprobs: StoredValue = None,
        enhancements: StoredValue = None, **params: StoredValue,
    ) -> None:
        params.pop("message", None)
        match finish_reason:
            case str() if finish_reason != "":
                params["finish_reason"] = map_finish_reason(finish_reason)
            case _other:
                params["finish_reason"] = None
        params["index"] = 0 if index is None else index
        match delta:
            case None:
                params["delta"] = Delta()
            case Delta():
                params["delta"] = delta
            case Mapping():
                params["delta"] = Delta(**delta)
            case _other:
                params["delta"] = delta
        params["logprobs"] = logprobs_value(logprobs)
        super().__init__(**params)
        if enhancements is not None:
            self["enhancements"] = enhancements


def native_finish_reason(
    fields: StoredValue, finish_reason: str,
) -> dict[str, StoredValue]:
    merged = provider_fields(fields)
    if merged is None:
        merged = {}
    merged["native_finish_reason"] = finish_reason
    return merged


def provider_fields(value: StoredValue) -> dict[str, StoredValue] | None:
    match value:
        case None:
            return None
        case Mapping():
            return {str(name): item for name, item in value.items()}
        case _other:
            raise ValidationError("Choices", ("provider_specific_fields",), "wrong type")


def message_value(message: StoredValue) -> StoredValue:
    match message:
        case None:
            return Message()
        case Message():
            return message
        case Mapping():
            return Message(**message)
        case _other:
            return message


def logprobs_value(logprobs: StoredValue) -> StoredValue:
    match logprobs:
        case Mapping():
            return ChoiceLogprobs(**logprobs)
        case _other:
            return logprobs

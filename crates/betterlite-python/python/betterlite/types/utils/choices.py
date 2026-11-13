import logging
from collections.abc import Mapping
from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import override

from betterlite.types.utils.base import (
    Dumpable,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
)
from betterlite.types.utils.message import Delta, Message
from betterlite.types.utils.tool_calls import ChoiceLogprobs

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
    logprobs: ChoiceLogprobs | JsonValue = None
    provider_specific_fields: dict[str, JsonValue] | None = None

    @override
    def __init__(
        self, finish_reason: JsonValue = None, index: JsonValue = None,
        message: ModelValue = None, logprobs: JsonValue = None,
        enhancements: JsonValue = None,
        provider_specific_fields: Mapping[str, ModelValue] | None = None,
        **params: InitValue,
    ) -> None:
        selected: dict[str, InitValue] | None = (
            None if provider_specific_fields is None
            else {
                str(name): dump_value(item, exclude_unset=True)
                for name, item in provider_specific_fields.items()
            }
        )
        if finish_reason is None:
            mapped: JsonValue = "stop"
        else:
            mapped = finish_reason
            match finish_reason:
                case str():
                    mapped = map_finish_reason(finish_reason)
                    if finish_reason != mapped:
                        selected = native_finish_reason(selected, finish_reason)
                case bool() | int() | float() | list() | dict():
                    mapped = finish_reason
        super().__init__(
            finish_reason=mapped, index=0 if index is None else index,
            message=message_value(message), logprobs=logprobs_value(logprobs),
            provider_specific_fields=selected, **params,
        )
        if enhancements is not None:
            self["enhancements"] = enhancements
        if self.logprobs is None:
            del self.logprobs
        if self.provider_specific_fields is None:
            del self.provider_specific_fields


class StreamingChoices(ResponseObject):
    finish_reason: ClassVar[FinishReason | None]
    index: ClassVar[int]
    delta: ClassVar[Delta]
    logprobs: ClassVar[ChoiceLogprobs | JsonValue]

    @override
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2007-L2038
        self, finish_reason: JsonValue = None, index: JsonValue = 0,
        delta: ModelValue = None, logprobs: JsonValue = None,
        enhancements: JsonValue = None, **params: InitValue,
    ) -> None:
        params.pop("message", None)
        reason: JsonValue = None
        if finish_reason is not None and finish_reason != "":
            match finish_reason:
                case str():
                    reason = map_finish_reason(finish_reason)
                case bool() | int() | float() | list() | dict():
                    reason = finish_reason
        selected_delta: InitValue = Delta()
        if delta is not None:
            match delta:
                case Delta():
                    selected_delta = delta
                case Dumpable():
                    selected_delta = Delta.model_validate(delta.model_dump())
                case Mapping():
                    selected_delta = Delta.model_validate(delta)
                case list() | bool() | int() | float() | str():
                    selected_delta = delta
        super().__init__(
            finish_reason=reason, index=0 if index is None else index,
            delta=selected_delta, logprobs=logprobs_value(logprobs), **params,
        )
        if enhancements is not None:
            self["enhancements"] = enhancements


def native_finish_reason(
    fields: dict[str, InitValue] | None, finish_reason: str,
) -> dict[str, InitValue]:
    merged: dict[str, InitValue] = {} if fields is None else dict(fields)
    merged["native_finish_reason"] = finish_reason
    return merged


def message_value(message: ModelValue) -> ModelValue:
    if message is None:
        return Message()
    match message:
        case Message():
            return message
        case Dumpable():
            dumped: dict[str, JsonValue] = message.model_dump()
            return Message.model_validate(dumped)
        case Mapping():
            return Message.model_validate(message)
        case list() | bool() | int() | float() | str():
            return message


def logprobs_value(logprobs: JsonValue) -> ModelValue:
    if logprobs is None:
        return None
    match logprobs:
        case Mapping():
            return ChoiceLogprobs.model_validate(logprobs)
        case list() | bool() | int() | float() | str():
            return logprobs

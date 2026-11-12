from collections.abc import Mapping
from typing import ClassVar

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject, dump_any
from betterlite.types.utils.finish_reason import FinishReason, map_finish_reason
from betterlite.types.utils.json_types import JsonValue
from betterlite.types.utils.message import Delta, Message
from betterlite.types.utils.tool_calls import ChoiceLogprobs
from betterlite.types.utils.values import StoredValue


class Choices(ResponseObject):
    finish_reason: FinishReason
    index: int
    message: Message
    logprobs: ChoiceLogprobs | JsonValue
    provider_specific_fields: dict[str, JsonValue] | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "logprobs": None,
        "provider_specific_fields": None,
    }

    @override
    def __init__(
        self,
        finish_reason: str | None = None,
        index: int | None = None,
        message: StoredValue = None,
        logprobs: StoredValue = None,
        enhancements: StoredValue = None,
        provider_specific_fields: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        fields = provider_specific_fields
        if finish_reason is None:
            params["finish_reason"] = "stop"
        else:
            mapped = map_finish_reason(finish_reason)
            params["finish_reason"] = mapped
            if finish_reason != mapped:
                fields = native_finish_reason(provider_specific_fields, finish_reason)
        params["index"] = 0 if index is None else index
        params["message"] = message_value(message)
        params["logprobs"] = logprobs_value(logprobs)
        super().__init__(**params)
        if enhancements is not None:
            self["enhancements"] = enhancements
        self.provider_specific_fields = json_mapping(fields)
        if self.logprobs is None:
            del self.logprobs
        if self.provider_specific_fields is None:
            del self.provider_specific_fields


class StreamingChoices(ResponseObject):
    finish_reason: FinishReason | None
    index: int
    delta: Delta
    logprobs: ChoiceLogprobs | JsonValue
    defaults: ClassVar[dict[str, StoredValue]] = {"logprobs": None}

    @override
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2007-L2038
        self,
        finish_reason: StoredValue = None,
        index: StoredValue = 0,
        delta: StoredValue = None,
        logprobs: StoredValue = None,
        enhancements: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        params.pop("message", None)
        match finish_reason:
            case str() if finish_reason != "":
                params["finish_reason"] = map_finish_reason(finish_reason)
            case _other:
                params["finish_reason"] = None
        params["index"] = index
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


def native_finish_reason(fields: StoredValue, finish_reason: str) -> StoredValue:
    merged = mapping_values(fields)
    merged["native_finish_reason"] = finish_reason
    return merged


def mapping_values(value: StoredValue) -> dict[str, StoredValue]:
    match value:
        case Mapping():
            return {str(name): item for name, item in value.items()}
        case _other:
            return {}


def json_mapping(value: StoredValue) -> dict[str, JsonValue] | None:
    match value:
        case Mapping():
            return {
                str(name): dump_any(entry, exclude_unset=False, exclude_none=False)
                for name, entry in value.items()
            }
        case _other:
            return None


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

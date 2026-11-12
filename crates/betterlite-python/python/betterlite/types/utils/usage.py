from collections.abc import Mapping
from typing import ClassVar, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import (
    MODEL_TYPES,
    JsonValue,
    ModelObject,
    StoredValue,
    ValidationError,
)

CACHED_TARGETS: tuple[tuple[Literal["cached", "write"], str], ...] = (
    ("cached", "prompt_cache_hit_tokens"), ("cached", "cache_read_input_tokens"),
    ("write", "cache_creation_input_tokens"),
)
PRIVATE_CACHE_PARAMS = (
    ("cache_creation_input_tokens", "_cache_creation_input_tokens"),
    ("cache_read_input_tokens", "_cache_read_input_tokens"),
    ("prompt_cache_hit_tokens", "_cache_read_input_tokens"),
)
DROP_WHEN_NONE = (
    "character_count", "image_count", "video_length_seconds", "audio_length_seconds",
    "query_count", "web_search_requests", "google_maps_grounding_requests", "tool_use_tokens",
    "cache_write_tokens", "cache_creation_tokens", "cache_creation_token_details",
    "cached_tokens_details",
)


class DeclaredOnly(ResponseObject):
    @override
    def __init__(self, **data: StoredValue) -> None:
        super().__init__(**{
            name: value for name, value in data.items() if name in type(self).__fields__
        })


class ServerToolUse(DeclaredOnly):
    web_search_requests: int | None
    tool_search_requests: int | None
    browser_open_requests: int | None

    @override
    def __getitem__(self, key: str) -> StoredValue:
        if key not in type(self).__fields__:
            raise KeyError(key)
        return super().__getitem__(key)


class CacheCreationTokenDetails(DeclaredOnly):
    ephemeral_5m_input_tokens: int | None
    ephemeral_1h_input_tokens: int | None


class CompletionTokensDetailsWrapper(ResponseObject):
    accepted_prediction_tokens: int | None
    audio_tokens: int | None
    reasoning_tokens: int | None
    rejected_prediction_tokens: int | None
    text_tokens: int | None
    image_tokens: int | None
    video_tokens: int | None


class PromptTokensDetailsWrapper(ResponseObject):
    audio_tokens: int | None
    cached_tokens: int | None
    text_tokens: int | None
    image_tokens: int | None
    video_tokens: int | None
    web_search_requests: int | None
    google_maps_grounding_requests: int | None
    tool_use_tokens: int | None
    character_count: int | None
    image_count: int | None
    video_length_seconds: float | None
    audio_length_seconds: float | None
    query_count: int | None
    cache_write_tokens: int | None
    cache_creation_tokens: int | None
    cache_creation_token_details: CacheCreationTokenDetails | None
    cached_tokens_details: JsonValue

    @override
    def __setattr__(self, name: str, value: StoredValue) -> None:
        super().__setattr__(name, value)
        if name == "cache_write_tokens":
            super().__setattr__("cache_creation_tokens", value)
        elif name == "cache_creation_tokens":
            super().__setattr__("cache_write_tokens", value)

    @override
    def __init__(self, **data: StoredValue) -> None:
        super().__init__(**data)
        write = self.cache_write_tokens
        if write is None:
            write = self.cache_creation_tokens
        if write is None and "cache_creation_input_tokens" in data:
            write = int_value(data["cache_creation_input_tokens"])
        self.cache_write_tokens = write
        drop_none_fields(self)


class Usage(ResponseObject):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    completion_tokens_details: CompletionTokensDetailsWrapper | None
    prompt_tokens_details: PromptTokensDetailsWrapper | None
    server_tool_use: ServerToolUse | None
    cost: float | None
    _cache_creation_input_tokens: int
    _cache_read_input_tokens: int
    defaults: ClassVar[dict[str, StoredValue]] = {
        "_cache_creation_input_tokens": 0,
        "_cache_read_input_tokens": 0,
    }

    @override
    def __init__(
        self, prompt_tokens: StoredValue = None, completion_tokens: StoredValue = None,
        total_tokens: StoredValue = None, reasoning_tokens: StoredValue = None,
        prompt_tokens_details: StoredValue = None, completion_tokens_details: StoredValue = None,
        server_tool_use: StoredValue = None, cost: StoredValue = None,
        **params: StoredValue,
    ) -> None:
        prompt_details_input = prompt_details_value(prompt_tokens_details)
        super().__init__(
            prompt_tokens=token_count(prompt_tokens, "prompt_tokens"),
            completion_tokens=token_count(completion_tokens, "completion_tokens"),
            total_tokens=token_count(total_tokens, "total_tokens"),
            completion_tokens_details=completion_tokens_details,
            prompt_tokens_details=prompt_details_input,
            server_tool_use=server_tool_use,
        )
        if reasoning_tokens is not None:
            self.completion_tokens_details = reasoning_details(
                self.completion_tokens_details, reasoning_tokens, completion_tokens,
            )
        prompt_details = self.prompt_tokens_details
        for kind, name in CACHED_TARGETS:
            cached = int_param(params, name)
            if cached is not None:
                prompt_details = token_details(prompt_details, cached, kind)
        self.prompt_tokens_details = prompt_details
        for name, target in PRIVATE_CACHE_PARAMS:
            value = int_param(params, name)
            if value is not None:
                self[target] = value
        if server_tool_use is None:
            del self.server_tool_use
        if cost is None:
            del self.cost
        else:
            self.cost = cost_number(cost)
        for name, item in params.items():
            self[name] = item


def int_value(value: StoredValue) -> int | None:
    match value:
        case int():
            return value
        case _other:
            return None


def prompt_details_value(value: StoredValue) -> StoredValue:
    if value is None or value.__class__ in MODEL_TYPES:
        return value
    match value:
        case ModelObject():
            dumped = value.model_dump()
            match dumped:
                case Mapping():
                    return dumped
                case _other:
                    raise ValidationError("Usage", ("prompt_tokens_details",), "wrong type")
        case _other:
            return value


def drop_none_fields(details: PromptTokensDetailsWrapper) -> None:
    for name in DROP_WHEN_NONE:
        if details[name] is None:
            del details[name]


def int_param(params: Mapping[str, StoredValue], name: str) -> int | None:
    if name not in params:
        return None
    return int_value(params[name])


def token_count(value: StoredValue, name: str) -> int:
    match value:
        case int():
            return value
        case None:
            return 0
        case _other:
            raise ValidationError("Usage", (name,), "expected an integer")


def cost_number(value: StoredValue) -> float:
    match value:
        case int() | float():
            return float(value)
        case _other:
            raise ValidationError("Usage", ("cost",), "expected a number")


def reasoning_details(
    details: CompletionTokensDetailsWrapper | None, reasoning_tokens: StoredValue,
    completion_tokens: StoredValue,
) -> CompletionTokensDetailsWrapper:
    reasoning = token_count(reasoning_tokens, "reasoning_tokens")
    wrapper = CompletionTokensDetailsWrapper() if details is None else details
    if wrapper.reasoning_tokens is None:
        wrapper.reasoning_tokens = reasoning
    if wrapper.text_tokens is None and completion_tokens is not None:
        calculated = token_count(completion_tokens, "completion_tokens") - reasoning
        calculated -= wrapper.image_tokens if wrapper.image_tokens is not None else 0
        calculated -= wrapper.audio_tokens if wrapper.audio_tokens is not None else 0
        wrapper.text_tokens = max(0, calculated)
    return wrapper


def token_details(
    details: PromptTokensDetailsWrapper | None, value: int, kind: Literal["cached", "write"],
) -> PromptTokensDetailsWrapper:
    target = PromptTokensDetailsWrapper() if details is None else details
    if kind == "cached":
        target.cached_tokens = value
    else:
        target.cache_write_tokens = value
    return target


def text_tokens_without_nested_reasoning(  # ast-grep-ignore: no-long-names
    completion_tokens: int, text_tokens: int, reasoning_tokens: int, other_modality_tokens: int,
) -> int:
    reported_total = text_tokens + reasoning_tokens + other_modality_tokens
    nested_reasoning = min(
        reasoning_tokens, text_tokens, max(reported_total - completion_tokens, 0),
    )
    return text_tokens - nested_reasoning

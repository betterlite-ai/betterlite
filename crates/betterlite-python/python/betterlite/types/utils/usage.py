from collections.abc import Mapping, Sequence
from typing import ClassVar, Literal

from pydantic import ConfigDict, JsonValue, PrivateAttr
from typing_extensions import override

from betterlite.types.utils.base import (
    Dumpable,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
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


class ServerToolUse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    web_search_requests: int | None = None
    tool_search_requests: int | None = None
    browser_open_requests: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    @override
    def __getitem__(self, key: str) -> InitValue:
        if key not in type(self).model_fields:
            raise KeyError(key)
        return super().__getitem__(key)


class CacheCreationTokenDetails(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    ephemeral_5m_input_tokens: int | None = None
    ephemeral_1h_input_tokens: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)


class CompletionTokensDetailsWrapper(ResponseObject):
    accepted_prediction_tokens: int | None = None
    audio_tokens: int | None = None
    reasoning_tokens: int | None = None
    rejected_prediction_tokens: int | None = None
    text_tokens: int | None = None
    image_tokens: int | None = None
    video_tokens: int | None = None


class PromptTokensDetailsWrapper(ResponseObject):
    audio_tokens: int | None = None
    cache_write_tokens: int | None = None
    cached_tokens: int | None = None
    text_tokens: int | None = None
    image_tokens: int | None = None
    video_tokens: int | None = None
    web_search_requests: int | None = None
    google_maps_grounding_requests: int | None = None
    tool_use_tokens: int | None = None
    character_count: int | None = None
    image_count: int | None = None
    video_length_seconds: float | None = None
    audio_length_seconds: float | None = None
    query_count: int | None = None
    cache_creation_tokens: int | None = None
    cache_creation_token_details: CacheCreationTokenDetails | None = None
    cached_tokens_details: JsonValue = None

    @override
    def __setattr__(self, name: str, value: InitValue) -> None:
        super().__setattr__(name, value)
        if name == "cache_write_tokens":
            super().__setattr__("cache_creation_tokens", value)
        elif name == "cache_creation_tokens":
            super().__setattr__("cache_write_tokens", value)

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)
        write = self.cache_write_tokens
        if write is None:
            write = self.cache_creation_tokens
        if write is None and "cache_creation_input_tokens" in data:
            write = int_value(data["cache_creation_input_tokens"])
        self.cache_write_tokens = write
        for name in DROP_WHEN_NONE:
            if name in self and self[name] is None:
                del self[name]


class Usage(ResponseObject):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    completion_tokens_details: CompletionTokensDetailsWrapper | None = None
    prompt_tokens_details: PromptTokensDetailsWrapper | None = None
    server_tool_use: ServerToolUse | None = None
    cost: float | None = None
    _cache_creation_input_tokens: int = PrivateAttr(default=0)
    _cache_read_input_tokens: int = PrivateAttr(default=0)

    @override
    def __init__(
        self, prompt_tokens: JsonValue = None, completion_tokens: JsonValue = None,
        total_tokens: JsonValue = None, reasoning_tokens: int | None = None,
        prompt_tokens_details: ModelValue = None,
        completion_tokens_details: ModelValue = None,
        server_tool_use: ModelValue | Mapping[str, JsonValue] = None,
        cost: JsonValue = None, **params: InitValue,
    ) -> None:
        counts: dict[str, InitValue] = {
            "prompt_tokens": token_count(prompt_tokens),
            "completion_tokens": token_count(completion_tokens),
            "total_tokens": token_count(total_tokens),
            "completion_tokens_details": dump_value(completion_tokens_details),
            "prompt_tokens_details": dump_value(prompt_tokens_details),
            "server_tool_use": server_tool_use,
            "cost": cost,
        }
        super().__init__(**counts)
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
        # LiteLLM leaves these two unset rather than passing them, and a reader sees the same fields
        for name in ("server_tool_use", "cost"):
            if self[name] is None:
                del self[name]
                self.model_fields_set.discard(name)
        for name, target in PRIVATE_CACHE_PARAMS:
            value = int_param(params, name)
            if value is not None:
                self[target] = value
        for name, item in params.items():
            self[name] = item


def int_value(value: InitValue) -> int | None:
    if value is None:
        return None
    match value:
        case int():
            return value
        case Dumpable() | Mapping() | Sequence() | float():
            return None


def int_param(params: Mapping[str, InitValue], name: str) -> int | None:
    if name not in params:
        return None
    return int_value(params[name])


def token_count(value: JsonValue) -> JsonValue:
    if value is None:
        return 0
    return value


def reasoning_details(
    details: CompletionTokensDetailsWrapper | None, reasoning_tokens: int,
    completion_tokens: JsonValue,
) -> CompletionTokensDetailsWrapper:
    wrapper = CompletionTokensDetailsWrapper() if details is None else details
    if wrapper.reasoning_tokens is None:
        wrapper.reasoning_tokens = reasoning_tokens
    count = int_value(completion_tokens)
    if wrapper.text_tokens is None and count is not None:
        calculated = count - reasoning_tokens
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

from collections.abc import Mapping
from typing import ClassVar

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.json_types import JsonValue
from betterlite.types.utils.values import StoredValue


class ServerToolUse(ResponseObject):
    web_search_requests: int | None
    tool_search_requests: int | None
    browser_open_requests: int | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "web_search_requests": None,
        "tool_search_requests": None,
        "browser_open_requests": None,
    }

    @override
    def __getitem__(self, key: str) -> StoredValue:
        values = self._values
        if key in values:
            return values[key]
        raise KeyError(key)


class CacheCreationTokenDetails(ResponseObject):
    ephemeral_5m_input_tokens: int | None
    ephemeral_1h_input_tokens: int | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "ephemeral_5m_input_tokens": None,
        "ephemeral_1h_input_tokens": None,
    }


class CompletionTokensDetailsWrapper(ResponseObject):
    accepted_prediction_tokens: int | None
    audio_tokens: int | None
    reasoning_tokens: int | None
    rejected_prediction_tokens: int | None
    text_tokens: int | None
    image_tokens: int | None
    video_tokens: int | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "accepted_prediction_tokens": None,
        "audio_tokens": None,
        "reasoning_tokens": None,
        "rejected_prediction_tokens": None,
        "text_tokens": None,
        "image_tokens": None,
        "video_tokens": None,
    }


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
    defaults: ClassVar[dict[str, StoredValue]] = {
        "audio_tokens": None,
        "cached_tokens": None,
        "text_tokens": None,
        "image_tokens": None,
        "video_tokens": None,
        "web_search_requests": None,
        "google_maps_grounding_requests": None,
        "tool_use_tokens": None,
        "character_count": None,
        "image_count": None,
        "video_length_seconds": None,
        "audio_length_seconds": None,
        "query_count": None,
        "cache_write_tokens": None,
        "cache_creation_tokens": None,
        "cache_creation_token_details": None,
        "cached_tokens_details": None,
    }

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
        mirror_cache_tokens(self, data)
        if self.cache_write_tokens is None and "cache_creation_input_tokens" in data:
            nested = int_value(data["cache_creation_input_tokens"])
            if nested is not None:
                self.cache_write_tokens = nested
        drop_none_fields(self)


def int_value(value: StoredValue) -> int | None:
    match value:
        case int():
            return value
        case _other:
            return None


def mirror_cache_tokens(
    details: PromptTokensDetailsWrapper,
    data: Mapping[str, StoredValue],
) -> None:
    if "cache_write_tokens" in data:
        write = int_value(data["cache_write_tokens"])
        if write is not None:
            details.cache_creation_tokens = write
    if "cache_creation_tokens" in data:
        creation = int_value(data["cache_creation_tokens"])
        if creation is not None:
            details.cache_write_tokens = creation


def drop_embedding_fields(details: PromptTokensDetailsWrapper) -> None:
    if details.character_count is None:
        del details.character_count
    if details.image_count is None:
        del details.image_count
    if details.video_length_seconds is None:
        del details.video_length_seconds
    if details.audio_length_seconds is None:
        del details.audio_length_seconds
    if details.query_count is None:
        del details.query_count


def drop_cache_fields(details: PromptTokensDetailsWrapper) -> None:
    if details.web_search_requests is None:
        del details.web_search_requests
    if details.google_maps_grounding_requests is None:
        del details.google_maps_grounding_requests
    if details.tool_use_tokens is None:
        del details.tool_use_tokens
    if details.cache_write_tokens is None:
        del details.cache_write_tokens
    if details.cache_creation_tokens is None:
        del details.cache_creation_tokens
    if details.cache_creation_token_details is None:
        del details.cache_creation_token_details
    if details.cached_tokens_details is None:
        del details.cached_tokens_details


def drop_none_fields(details: PromptTokensDetailsWrapper) -> None:
    drop_embedding_fields(details)
    drop_cache_fields(details)

import asyncio
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import image_handling
from litellm.litellm_core_utils.prompt_templates.image_handling import (
    RemoteMedia,
    _remote_urls_to_inline,  # pyright: ignore[reportPrivateUsage]
    async_inline_remote_media,
    inline_every_remote_url,
    inline_remote_image_urls,
    inline_remote_media,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.utils import get_base64_str, is_base64_encoded

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Predicate: TypeAlias = Callable[[RemoteMedia], bool]


class _MappedConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


class _MappedAsyncConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    async def __call__(self, url: str) -> str:
        return self.mapping[url]


def _messages(input_value: dict[str, Json]) -> list[AllMessageValues]:
    return cast("list[AllMessageValues]", input_value["messages"])


def _mapping(input_value: dict[str, Json]) -> dict[str, str]:
    return cast("dict[str, str]", input_value["data_urls"])


def _predicate(input_value: dict[str, Json]) -> Predicate:
    if input_value.get("should_inline") == "image_urls":
        return inline_remote_image_urls
    return inline_every_remote_url


def _sync(messages: list[AllMessageValues], mapping: dict[str, str], predicate: Predicate) -> Json:
    with patch.object(image_handling, "convert_url_to_base64", _MappedConvert(mapping)):  # pyright: ignore[reportPrivateUsage]
        return cast(Json, inline_remote_media(messages, should_inline=predicate))


async def _async(
    messages: list[AllMessageValues], mapping: dict[str, str], predicate: Predicate
) -> Json:
    with patch.object(image_handling, "async_convert_url_to_base64", _MappedAsyncConvert(mapping)):  # pyright: ignore[reportPrivateUsage]
        return cast(Json, await async_inline_remote_media(messages, should_inline=predicate))


def _inline(input_value: dict[str, Json]) -> Json:
    messages = _messages(input_value)
    mapping = _mapping(input_value)
    predicate = _predicate(input_value)
    urls = list(_remote_urls_to_inline(messages, predicate))
    if input_value.get("mode") == "concurrent":
        messages = asyncio.run(_async(messages, mapping, predicate))  # pyright: ignore[reportArgumentType]
    else:
        messages = _sync(messages, mapping, predicate)
    return cast(Json, {"messages": messages, "urls": urls})


def _fetch(input_value: dict[str, Json]) -> Json:
    return cast(Json, {"messages": inline_remote_media(_messages(input_value))})


def _base64_strings(input_value: dict[str, Json]) -> Json:
    values = cast("list[str]", input_value["values"])
    return [get_base64_str(value) for value in values]


def _classify(input_value: dict[str, Json]) -> Json:
    values = cast("list[str]", input_value["values"])
    return [is_base64_encoded(value) for value in values]


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("call"):
        case "inline":
            return _inline(input_value)
        case "fetch":
            return _fetch(input_value)
        case "get_base64_str":
            return _base64_strings(input_value)
        case "is_base64_encoded":
            return _classify(input_value)
        case other:
            raise ValueError(f"unknown inline_media call: {other!r}")

import asyncio
import base64
from typing import TypeAlias, cast

import httpx
from litellm.litellm_core_utils.prompt_templates.image_handling import (
    _process_image_response,  # pyright: ignore[reportPrivateUsage]
    async_convert_url_to_base64,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _body(input_value: dict[str, Json]) -> bytes:
    repeat = input_value.get("body_repeat")
    if isinstance(repeat, dict):
        return bytes([cast(int, repeat["byte"])]) * cast(int, repeat["count"])
    return base64.b64decode(cast(str, input_value.get("body_base64") or ""))


def _read_response(input_value: dict[str, Json]) -> Json:
    headers = cast(dict[str, str], input_value.get("headers") or {})
    body = httpx.ByteStream(_body(input_value))
    response = httpx.Response(
        status_code=cast(int, input_value["status"]),
        headers=headers,
        stream=body,
    )
    return _process_image_response(response, cast(str, input_value["url"]))


def _convert(input_value: dict[str, Json]) -> Json:
    return asyncio.run(async_convert_url_to_base64(cast(str, input_value["url"])))


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("call"):
        case "read_response":
            return _read_response(input_value)
        case "convert":
            return _convert(input_value)
        case other:
            raise ValueError(f"unknown b5083 call: {other!r}")

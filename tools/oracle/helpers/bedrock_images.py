import base64
import json
from typing import TypeAlias, cast

import httpx
from litellm.litellm_core_utils.prompt_templates.factory import (
    BedrockImageProcessor,
    _parse_content_type,  # pyright: ignore[reportPrivateUsage]
    _parse_mime_type,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _text(input_value: dict[str, Json], key: str) -> str:
    return cast(str, input_value[key])


def _process(input_value: dict[str, Json]) -> Json:
    declared_format = input_value.get("format")
    format_value = declared_format if isinstance(declared_format, str) else None
    block = BedrockImageProcessor.process_image_sync(
        image_url=_text(input_value, "image_url"), format=format_value
    )
    return cast(Json, json.loads(json.dumps(block)))


def _post_call(input_value: dict[str, Json]) -> Json:
    headers = cast(dict[str, str], input_value.get("headers") or {})
    body = base64.b64decode(cast(str, input_value.get("body_base64") or ""))
    response = httpx.Response(status_code=200, headers=headers, content=body)
    base64_bytes, content_type = BedrockImageProcessor._post_call_image_processing(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        response, _text(input_value, "image_url")
    )
    return cast(Json, [base64_bytes, content_type])


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("call"):
        case "process":
            return _process(input_value)
        case "parse_content_type":
            return _parse_content_type(_text(input_value, "content_type"))
        case "parse_mime_type":
            return _parse_mime_type(_text(input_value, "base64_data"))
        case "post_call":
            return _post_call(input_value)
        case other:
            raise ValueError(f"unknown bedrock_images call: {other!r}")

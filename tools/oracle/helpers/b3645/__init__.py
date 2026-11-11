from typing import TypeAlias

import httpx
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    transcribe_passthrough_logging_handler as handler,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def string_value(input_value: dict[str, Json], name: str) -> str:
    value = input_value[name]
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def number_value(input_value: dict[str, Json], name: str) -> float:
    value = input_value[name]
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise ValueError(f"{name} must be a number")
    return float(value)


def header_value(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError("each header value must be a string")
    return value


def header_map(input_value: dict[str, Json]) -> httpx.Headers:
    value = input_value["headers"]
    if not isinstance(value, dict):
        raise ValueError("headers must be an object")
    return httpx.Headers(
        {name: header_value(entry) for name, entry in value.items()}
    )


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value, "function")
    if function == "error_type":
        body = string_value(input_value, "body")
        return handler.aws_error_type(httpx.Response(400, content=body.encode()))
    if function == "s3_url":
        return handler.s3_media_url(
            string_value(input_value, "media_uri"),
            string_value(input_value, "region"),
        )
    return handler.media_predates_job(
        header_map(input_value), number_value(input_value, "job_created_at")
    )

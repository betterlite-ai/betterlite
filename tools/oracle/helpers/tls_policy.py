import os
from typing import TypeAlias, cast

import litellm
from litellm.llms.custom_httpx.http_handler import (
    get_default_headers,  # pyright: ignore[reportUnknownVariableType]
    get_ssl_verify,
    http2_enabled,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FIELD_COUNT = 3


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {
            "ssl_verify": verify,
            "http2": bool(http2),
            "environment": dict(environment),
        } if len(input_value) == FIELD_COUNT:
            pass
        case _:
            raise ValueError("tls_policy input has missing, extra or mistyped fields")
    os.environ.clear()
    for name, value in environment.items():
        if not isinstance(value, str):
            raise ValueError("environment values must be strings")
        os.environ[name] = value
    litellm.http2 = http2
    resolved = get_ssl_verify(verify if isinstance(verify, (bool, str)) else None)
    raw_headers = cast(dict[str, str], get_default_headers())
    headers = cast(dict[str, Json], raw_headers)
    result: dict[str, Json] = {
        "ssl_verify": resolved,
        "http2": http2_enabled(),
        "headers": headers,
    }
    return result

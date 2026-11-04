import os
from typing import TypeAlias, cast

import litellm
from litellm.llms.custom_httpx.http_handler import (
    _create_ssl_context,  # pyright: ignore[reportPrivateUsage]
    get_default_headers,  # pyright: ignore[reportUnknownVariableType]
    get_ssl_verify,
    http2_enabled,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FIELD_COUNT = 4


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {
            "deployment_ssl_verify": deployment_verify,
            "config_ssl_verify": config_verify,
            "config_http2": config_http2,
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
    if deployment_verify is not None and not isinstance(deployment_verify, bool):
        raise ValueError("deployment_ssl_verify must be a bool or null")
    if config_verify is not None and not isinstance(config_verify, bool):
        raise ValueError("config_ssl_verify must be a bool or null")
    if config_http2 is not None and not isinstance(config_http2, bool):
        raise ValueError("config_http2 must be a bool or null")
    litellm.ssl_verify = config_verify
    litellm.http2 = config_http2
    resolved = get_ssl_verify(deployment_verify)
    curve = os.environ.get("SSL_ECDH_CURVE")
    if curve is not None:
        _create_ssl_context(None, None, curve)
    raw_headers = cast(dict[str, str], get_default_headers())
    headers = cast(dict[str, Json], raw_headers)
    certificate = os.environ.get("SSL_CERTIFICATE")
    ca_file = os.environ.get("SSL_CERT_FILE")
    result: dict[str, Json] = {
        "ssl_verify": resolved,
        "http2": http2_enabled(),
        "user_agent": headers.get("User-Agent", ""),
        "certificate": certificate,
        "ca_file": ca_file,
        "ecdh_curve": curve,
    }
    return result

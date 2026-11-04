import os
import ssl
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.custom_httpx.http_handler import (
    get_default_headers,  # pyright: ignore[reportUnknownVariableType]
    get_ssl_verify,
    http2_enabled,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FIELD_COUNT = 5


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {
            "ca_pem": ca_pem,
            "deployment_ssl_verify": deployment_verify,
            "config_ssl_verify": config_verify,
            "config_http2": config_http2,
            "environment": dict(environment),
        } if len(input_value) == FIELD_COUNT:
            pass
        case _:
            raise ValueError("tls_policy input has missing, extra or mistyped fields")
    if ca_pem is not None and not isinstance(ca_pem, str):
        raise ValueError("ca_pem must be a string or null")
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
    litellm.ssl_verify = config_verify if config_verify is not None else True
    litellm.http2 = config_http2
    # The helper models existence from the supplied path, without reading host files.
    def exists(path: str) -> bool:
        return ca_pem is not None and path == environment.get("SSL_CERT_FILE")

    with patch("os.path.exists", side_effect=exists):
        resolved = get_ssl_verify(deployment_verify)
    curve = os.environ.get("SSL_ECDH_CURVE")
    raw_headers = cast(dict[str, str], get_default_headers())
    headers = cast(dict[str, Json], raw_headers)
    certificate = os.environ.get("SSL_CERTIFICATE")
    ca_file = os.environ.get("SSL_CERT_FILE")
    result: dict[str, Json] = {
        "ssl_verify": resolved,
        "ca_valid": valid_ca(ca_pem) if ca_pem is not None else None,
        "http2": http2_enabled(),
        "user_agent": headers.get("User-Agent", ""),
        "certificate": certificate,
        "ca_file": ca_file,
        "ecdh_curve": curve,
    }
    return result


def valid_ca(pem: str) -> bool:
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    try:
        context.load_verify_locations(cadata=pem)
    except ssl.SSLError:
        return False
    return True

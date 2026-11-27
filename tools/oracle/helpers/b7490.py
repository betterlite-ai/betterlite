from typing import Final, NotRequired, TypedDict
from urllib.parse import urlsplit

from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import (
    MCPRequestHandler,
    _gateway_dcr_challenge,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L278-L303
    _has_client_supplied_mcp_auth,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L201-L205
    _is_gateway_dcr_challenge_scope,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L252-L276
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException
from starlette.requests import Request
from starlette.types import Scope

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HTTP_PORT: Final = 80
HTTPS_PORT: Final = 443


class Input(TypedDict):
    function: str
    mcp_auth_header: NotRequired[str]
    server_headers: NotRequired[dict[str, dict[str, str]]]
    route: NotRequired[str]
    mcp_servers: NotRequired[list[str]]
    invalid_token: NotRequired[bool]
    requested_name: NotRequired[str]
    base_url: NotRequired[str]
    path: NotRequired[str]
    root_path: NotRequired[str]
    server_root_path: NotRequired[str]
    error_status: NotRequired[int]


def request_for(input_value: Input) -> Request:
    parts = urlsplit(input_value["base_url"])
    host = parts.hostname
    if host is None:
        raise ValueError("base_url must carry a host")
    port = parts.port
    if port is None:
        port = HTTPS_PORT if parts.scheme == "https" else HTTP_PORT
    scope: Scope = {
        "type": "http",
        "http_version": "1.1",
        "method": "POST",
        "scheme": parts.scheme,
        "server": (host, port),
        "path": input_value["path"],
        "query_string": b"",
        "headers": [],
    }
    root_path = input_value.get("root_path")
    if root_path is not None:
        scope["root_path"] = root_path
    return Request(scope)


def auth_headers(input_value: Input) -> dict[str, dict[str, str]]:
    headers = input_value.get("server_headers")
    if headers is None:
        return {}
    return headers


def exception_value(error: StarletteHTTPException) -> JsonValue:
    headers = error.headers
    if headers is None:
        headers = {}
    outcome = {
        "status": error.status_code,
        "detail": error.detail,
        "headers": [[name, value] for name, value in headers.items()],
    }
    return JSON.validate_python(outcome)


def client_auth_value(input_value: Input) -> JsonValue:
    value = _has_client_supplied_mcp_auth(
        input_value.get("mcp_auth_header"),
        auth_headers(input_value),
    )
    return {"value": value}


def gateway_value(input_value: Input) -> JsonValue:
    request = request_for(input_value)
    error = _gateway_dcr_challenge(
        request,
        input_value["route"],
        input_value.get("mcp_servers"),
        input_value["invalid_token"],
    )
    return exception_value(error)


def scope_value(input_value: Input) -> JsonValue:
    error = HTTPException(status_code=input_value["error_status"])
    value = _is_gateway_dcr_challenge_scope(
        route=input_value["route"],
        mcp_servers=input_value.get("mcp_servers"),
        mcp_auth_header=input_value.get("mcp_auth_header"),
        mcp_server_auth_headers=auth_headers(input_value),
        exc=error,
        client_ip=None,
    )
    return {"value": value}


def bridge_value(input_value: Input) -> JsonValue:
    request = request_for(input_value)
    error = MCPRequestHandler._dcr_bridge_invalid_token_challenge(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L958-L976
        requested_name=input_value["requested_name"],
        request=request,
    )
    return exception_value(error)


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "client_auth":
        return client_auth_value(input_value)
    if function == "gateway_challenge":
        return gateway_value(input_value)
    if function == "bridge_challenge":
        return bridge_value(input_value)
    if function == "challenge_scope":
        return scope_value(input_value)
    raise ValueError(f"unknown b7490 function: {function}")

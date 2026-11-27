from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict
from urllib.parse import urlsplit

from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import (
    MCPRequestHandler,
    _gateway_dcr_challenge,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L278-L303
    _has_client_supplied_mcp_auth,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L201-L205
    _is_gateway_dcr_challenge_scope,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L252-L276
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException
from starlette.requests import Request
from starlette.types import Scope

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HTTP_PORT: Final = 80
HTTPS_PORT: Final = 443


class ClientAuthInput(TypedDict):
    function: Literal["client_auth"]
    mcp_auth_header: NotRequired[str | None]
    server_headers: NotRequired[dict[str, dict[str, str]] | None]


class GatewayChallengeInput(TypedDict):
    function: Literal["gateway_challenge"]
    route: str
    mcp_servers: NotRequired[list[str] | None]
    invalid_token: bool
    base_url: str
    path: str
    root_path: NotRequired[str | None]


class BridgeChallengeInput(TypedDict):
    function: Literal["bridge_challenge"]
    requested_name: str
    base_url: str
    path: str
    root_path: NotRequired[str | None]


class ChallengeScopeInput(TypedDict):
    function: Literal["challenge_scope"]
    route: str
    mcp_servers: NotRequired[list[str] | None]
    mcp_auth_header: NotRequired[str | None]
    server_headers: NotRequired[dict[str, dict[str, str]] | None]
    error_status: int


Input: TypeAlias = (
    ClientAuthInput | GatewayChallengeInput | BridgeChallengeInput | ChallengeScopeInput
)


def request_for(input_value: GatewayChallengeInput | BridgeChallengeInput) -> Request:
    parts = urlsplit(input_value["base_url"])
    host = parts.hostname
    if host is None:
        raise ValueError("base_url must carry a host")
    port = parts.port
    if port is None:
        port = HTTPS_PORT if parts.scheme == "https" else HTTP_PORT
    headers: list[tuple[bytes, bytes]] = []
    scope: Scope = {
        "type": "http",
        "http_version": "1.1",
        "method": "POST",
        "scheme": parts.scheme,
        "server": (host, port),
        "path": input_value["path"],
        "query_string": b"",
        "headers": headers,
    }
    root_path = input_value.get("root_path")
    if root_path is not None:
        scope["root_path"] = root_path
    return Request(scope)


def exception_value(error: StarletteHTTPException) -> JsonValue:
    headers: Mapping[str, str] = {} if error.headers is None else error.headers
    outcome = {
        "status": error.status_code,
        "detail": error.detail,
        "headers": [[name, value] for name, value in headers.items()],
    }
    return JSON.validate_python(outcome)


def client_auth_value(input_value: ClientAuthInput) -> JsonValue:
    supplied = _has_client_supplied_mcp_auth(
        input_value.get("mcp_auth_header"),
        input_value.get("server_headers"),
    )
    return {"value": supplied}


def gateway_value(input_value: GatewayChallengeInput) -> JsonValue:
    error = _gateway_dcr_challenge(
        request_for(input_value),
        input_value["route"],
        input_value.get("mcp_servers"),
        input_value["invalid_token"],
    )
    return exception_value(error)


def scope_value(input_value: ChallengeScopeInput) -> JsonValue:
    error = HTTPException(status_code=input_value["error_status"])
    value = _is_gateway_dcr_challenge_scope(
        route=input_value["route"],
        mcp_servers=input_value.get("mcp_servers"),
        mcp_auth_header=input_value.get("mcp_auth_header"),
        mcp_server_auth_headers=input_value.get("server_headers"),
        exc=error,
        client_ip=None,
    )
    return {"value": value}


def bridge_value(input_value: BridgeChallengeInput) -> JsonValue:
    error = MCPRequestHandler._dcr_bridge_invalid_token_challenge(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L958-L976
        requested_name=input_value["requested_name"],
        request=request_for(input_value),
    )
    return exception_value(error)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "client_auth":
        return client_auth_value(input_value)
    if input_value["function"] == "gateway_challenge":
        return gateway_value(input_value)
    if input_value["function"] == "bridge_challenge":
        return bridge_value(input_value)
    return scope_value(input_value)

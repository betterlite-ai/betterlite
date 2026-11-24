from collections.abc import Mapping

from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    raise_token_exchange_challenge,
    raise_user_oauth_challenge,
)
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue
from starlette.exceptions import HTTPException as RaisedError

from b4667.wire import JSON_ADAPTER


def exception_answer(exception: RaisedError) -> dict[str, JsonValue]:
    headers: Mapping[str, str] = exception.headers if exception.headers is not None else {}
    lowered: dict[str, str] = {name.lower(): value for name, value in headers.items()}
    return {
        "status_code": exception.status_code,
        "headers": JSON_ADAPTER.validate_python(lowered),
    }


def user_challenge_answer(server: MCPServer, root_path: str) -> dict[str, JsonValue]:
    try:
        raise_user_oauth_challenge(server, root_path=root_path)
    except RaisedError as exception:
        return exception_answer(exception)


def exchange_challenge_answer(
    server: MCPServer, root_path: str, claims: str | None
) -> dict[str, JsonValue]:
    try:
        raise_token_exchange_challenge(server, root_path=root_path, claims=claims)
    except RaisedError as exception:
        return exception_answer(exception)

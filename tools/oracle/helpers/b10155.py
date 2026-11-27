from typing import Final, Literal, TypedDict

from litellm.proxy._experimental.mcp_server.operations import (
    _client_has_passthrough_authorization,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/operations.py#L714-L731
    _client_has_per_server_auth_header,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/operations.py#L681-L711
)
from litellm.proxy._experimental.mcp_server.utils import (
    lookup_mcp_server_auth_in_headers,
    sanitize_mcp_alias_for_header,
)
from litellm.types.mcp import MCPTransport
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter

JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ServerSpec(TypedDict):
    alias: str | None
    server_name: str | None
    access_groups: list[str] | None


class SanitizeCall(TypedDict):
    function: Literal["sanitize_alias"]
    alias: str


class LookupCall(TypedDict):
    function: Literal["lookup"]
    headers: dict[str, dict[str, str]]
    alias: str | None
    server_name: str | None
    access_groups: list[str]


class PerServerCall(TypedDict):
    function: Literal["per_server_authorization"]
    server: ServerSpec
    server_auth_headers: dict[str, dict[str, str]] | None


class PassthroughCall(TypedDict):
    function: Literal["passthrough_authorization"]
    server: ServerSpec
    server_auth_headers: dict[str, dict[str, str]] | None
    oauth2_headers: dict[str, str] | None


Call = SanitizeCall | LookupCall | PerServerCall | PassthroughCall


class Input(TypedDict):
    calls: list[Call]


def build_server(spec: ServerSpec) -> MCPServer:
    return MCPServer(
        server_id="test-server",
        name="test-server",
        transport=MCPTransport.http,
        alias=spec["alias"],
        server_name=spec["server_name"],
        access_groups=spec["access_groups"],
    )


def lookup_answer(call: LookupCall) -> JsonValue:
    resolved = lookup_mcp_server_auth_in_headers(
        call["headers"],
        alias=call["alias"],
        server_name=call["server_name"],
        access_groups=call["access_groups"],
    )
    return {"resolved": JSON_VALUE.validate_python(resolved)}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "sanitize_alias":
            return {"sanitized": sanitize_mcp_alias_for_header(call["alias"])}
        case "lookup":
            return lookup_answer(call)
        case "per_server_authorization":
            return {
                "has_header": _client_has_per_server_auth_header(
                    build_server(call["server"]), call["server_auth_headers"]
                )
            }
        case "passthrough_authorization":
            return {
                "has_passthrough": _client_has_passthrough_authorization(
                    build_server(call["server"]),
                    call["oauth2_headers"],
                    call["server_auth_headers"],
                )
            }


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"answers": answers}

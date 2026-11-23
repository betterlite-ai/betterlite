from typing import TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    validate_static_credential,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import Error, Ok
from litellm.types.mcp import MCPAuthType
from pydantic import TypeAdapter


class Input(TypedDict):
    auth_type: str
    headers: dict[str, str]
    upstream_token_header: str | None
    static_header_names: list[str]


class Answer(TypedDict):
    ok: bool
    error: str | None


def run(input_value: Input) -> Answer:
    result = validate_static_credential(
        TypeAdapter(MCPAuthType).validate_python(input_value["auth_type"]),
        input_value["headers"],
        input_value["upstream_token_header"],
        input_value["static_header_names"],
    )
    match result:
        case Ok():
            return Answer(ok=True, error=None)
        case Error(error):
            return Answer(ok=False, error=error.tag)

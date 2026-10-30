import json
from typing import cast

from litellm.types.mcp import (
    MCPAuthType,
    MCPCredentials,
    MCPTokenEndpointAuthMethod,
    MCPTransportType,
)
from litellm.types.mcp_server.mcp_server_manager import MCPInfo, parse_pinned_tools
from pydantic import TypeAdapter, ValidationError


def invoke(input_value: dict[str, object]) -> object:
    function = input_value["function"]
    value = input_value["value"]
    if function == "credential_contract":
        return TypeAdapter[object](MCPCredentials).validate_python(value)
    if function == "transport":
        return TypeAdapter[object](MCPTransportType).validate_python(value)
    if function == "auth":
        return TypeAdapter[object](MCPAuthType).validate_python(value)
    if function == "token_endpoint_method":
        return TypeAdapter[object](MCPTokenEndpointAuthMethod).validate_python(value)
    if function == "pinned_tool_contract":
        tools = parse_pinned_tools(value)
        return None if tools is None else {
            name: cast(dict[str, object], tool.model_dump()) for name, tool in tools.items()
        }
    if function == "protocol_metadata":
        return TypeAdapter[object](MCPInfo).validate_python(value)
    raise ValueError(f"unknown function: {function}")


def run(input_value: dict[str, object]) -> object:
    steps = cast(dict[str, dict[str, object]], input_value["steps"])
    results: dict[str, object] = {}
    for name, step in steps.items():
        try:
            results[name] = {"value": invoke(step), "error": None}
        except (ValidationError, json.JSONDecodeError) as error:
            results[name] = {"value": None, "error": type(error).__name__}
    return results

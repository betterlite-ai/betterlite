import asyncio
import json
from typing import TypedDict

import pytest
from mcp_server import NORMAL, REFUSE, SLOW, UNAUTHORIZED_MODE, Server
from pydantic import JsonValue, TypeAdapter

from betterlite import _native

BEARER = "tok-1"
DEFAULT_TIMEOUT = 5.0
SHORT_TIMEOUT = 0.2
PAGES = 2
FINGERPRINT_LENGTH = 64
INTERNAL_ERROR = -32603
DENIED_CODE = -32602
UPDATED_AUTH = "next-value"


class Tool(TypedDict):
    name: str
    description: str | None
    inputSchema: dict[str, JsonValue] | None


class OpenAiFunction(TypedDict):
    name: str
    description: str
    parameters: dict[str, JsonValue]
    strict: bool


class OpenAiTool(TypedDict):
    type: str
    function: OpenAiFunction


class ResponsesTool(TypedDict):
    name: str
    description: str
    parameters: dict[str, JsonValue]
    strict: bool
    type: str


class Page(TypedDict):
    tools: list[Tool]
    nextCursor: str | None


class Prompt(TypedDict):
    name: str


class Resource(TypedDict):
    uri: str


class ResourceTemplate(TypedDict):
    uriTemplate: str


class TextBlock(TypedDict):
    type: str
    text: str


class ToolResult(TypedDict):
    content: list[TextBlock]
    isError: bool


def settings_for(server: Server, *, timeout: float = DEFAULT_TIMEOUT) -> str:
    settings: dict[str, JsonValue] = {
        "server_url": server.url,
        "auth_type": "bearer_token",
        "auth_value": BEARER,
        "timeout": timeout,
    }
    return json.dumps(settings)


def client_for(server: Server, *, timeout: float = DEFAULT_TIMEOUT) -> _native.NativeMcpClient:
    settings = settings_for(server, timeout=timeout)
    return _native.NativeMcpClient(settings)


def pages_of(text: str) -> Page:
    adapter = TypeAdapter(Page)
    return adapter.validate_json(text)


def tool_result_of(text: str) -> ToolResult:
    adapter = TypeAdapter(ToolResult)
    return adapter.validate_json(text)


def error_of(text: str) -> dict[str, JsonValue]:
    adapter = TypeAdapter(dict[str, JsonValue])
    return adapter.validate_json(text)


async def call_then_instructions(
    client: _native.NativeMcpClient, request: str
) -> tuple[str, str | None]:
    text = await client.call_tool(request)
    return text, client.instructions()


async def pages_flow(server: Server) -> tuple[list[Tool], list[str], str | None]:
    client = client_for(server)
    text = await client.list_tools()
    tools = TypeAdapter(list[Tool]).validate_json(text)
    return tools, server.record.methods, server.record.headers.get("authorization")


async def prompt_flow(server: Server) -> tuple[str, str]:
    client = client_for(server)
    prompts = await client.list_prompts()
    request: dict[str, JsonValue] = {"name": "greet"}
    prompt = await client.get_prompt(json.dumps(request))
    return prompts, prompt


async def resource_flow(server: Server) -> tuple[str, str, str]:
    client = client_for(server)
    resources = await client.list_resources()
    templates = await client.list_resource_templates()
    read = await client.read_resource("file:///notes.txt")
    return resources, templates, read


async def fingerprint_flow(server: Server) -> tuple[str, str, str | None]:
    client = client_for(server)
    first = await client.discovery_auth_fingerprint()
    await client.update_auth_value(json.dumps(UPDATED_AUTH))
    second = await client.discovery_auth_fingerprint()
    await client.list_tools()
    header = server.record.headers.get("authorization")
    return first, second, header


async def swallowed_flow(server: Server) -> tuple[str, str]:
    client = client_for(server)
    tools = await client.list_tools()
    request: dict[str, JsonValue] = {"name": "search"}
    result = await client.call_tool(json.dumps(request))
    return tools, result


async def list_failure(client: _native.NativeMcpClient) -> str:
    try:
        await client.list_tools(raise_on_error=True)
    except _native.NativeMcpError as raised:
        return str(raised)
    raise AssertionError("the server failure became a result")


async def call_failure(client: _native.NativeMcpClient, request: str) -> str:
    try:
        await client.call_tool(request)
    except _native.NativeMcpError as raised:
        return str(raised)
    raise AssertionError("the server failure became a result")


async def session_flow(server: Server) -> tuple[Page, Page, ToolResult, ToolResult, str]:
    client = client_for(server)
    session = await client.open_session()
    first = pages_of(await session.list_tools_page())
    second = pages_of(await session.list_tools_page("page2"))
    call_arguments: dict[str, JsonValue] = {}
    call_request: dict[str, JsonValue] = {"name": "search", "arguments": call_arguments}
    called = tool_result_of(await session.call_tool(json.dumps(call_request)))
    function_arguments: dict[str, JsonValue] = {"key": "value"}
    function: dict[str, JsonValue] = {
        "name": "fetch",
        "arguments": json.dumps(function_arguments),
    }
    openai_request: dict[str, JsonValue] = {"function": function}
    openai = tool_result_of(await session.call_openai_tool(json.dumps(openai_request)))
    await session.aclose()
    closed: str | None = None
    try:
        await session.list_tools_page()
    except _native.NativeMcpError as raised:
        closed = str(raised)
    if closed is None:
        raise AssertionError("a closed session answered a call")
    return first, second, called, openai, closed


async def load_both(server: Server) -> tuple[str, str]:
    client = client_for(server)
    session = await client.open_session()
    mcp = await session.load_tools("mcp")
    openai = await session.load_tools("openai")
    await session.aclose()
    return mcp, openai


async def open_a_session(server: Server) -> _native.NativeMcpSession:
    client = client_for(server)
    return await client.open_session()


def test_client_calls_pages() -> None:
    with Server() as server:
        tools, listed, authorization = asyncio.run(pages_flow(server))
    assert [tool["name"] for tool in tools] == ["search", "fetch"]
    assert listed.count("tools/list") == PAGES
    assert authorization == f"Bearer {BEARER}"


def test_client_calls_tool() -> None:
    with Server() as server:
        client = client_for(server)
        assert client.instructions() is None
        arguments: dict[str, JsonValue] = {"q": "x"}
        request: dict[str, JsonValue] = {"name": "search", "arguments": arguments}
        text, instructions = asyncio.run(call_then_instructions(client, json.dumps(request)))
    result = tool_result_of(text)
    assert result["content"][0]["text"] == "called search"
    assert instructions == "follow the map"


def test_client_calls_prompts() -> None:
    with Server() as server:
        prompts, prompt = asyncio.run(prompt_flow(server))
        resources, templates, read = asyncio.run(resource_flow(server))
    found_prompts = TypeAdapter(list[Prompt]).validate_json(prompts)
    found_resources = TypeAdapter(list[Resource]).validate_json(resources)
    found_templates = TypeAdapter(list[ResourceTemplate]).validate_json(templates)
    assert found_prompts[0]["name"] == "greet"
    assert "hi there" in prompt
    assert found_resources[0]["uri"] == "file:///notes.txt"
    assert found_templates[0]["uriTemplate"] == "file:///{name}"
    assert "body" in read


def test_client_calls_fingerprint() -> None:
    with Server() as server:
        first, second, authorization = asyncio.run(fingerprint_flow(server))
    assert len(first) == FINGERPRINT_LENGTH
    assert first != second
    assert authorization == f"Bearer {UPDATED_AUTH}"


def test_client_calls_swallowed() -> None:
    with Server(mode=REFUSE) as server:
        tools, result = asyncio.run(swallowed_flow(server))
    assert TypeAdapter(list[Tool]).validate_json(tools) == []
    assert tool_result_of(result)["isError"] is True


def test_transforms_shapes() -> None:
    properties: dict[str, JsonValue] = {}
    schema: dict[str, JsonValue] = {"type": "object", "properties": properties}
    tool: dict[str, JsonValue] = {
        "name": "search",
        "description": "finds things",
        "inputSchema": schema,
    }
    openai = TypeAdapter(OpenAiTool).validate_json(
        _native.tool_to_openai(json.dumps(tool))
    )
    responses = TypeAdapter(ResponsesTool).validate_json(
        _native.tool_to_responses(json.dumps(tool))
    )
    anthropic = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.tool_to_anthropic(json.dumps(tool))
    )
    assert openai["type"] == "function"
    assert openai["function"]["name"] == "search"
    assert responses["name"] == "search"
    assert responses["type"] == "function"
    assert anthropic["name"] == "search"


def test_transforms_schema() -> None:
    properties: dict[str, JsonValue] = {}
    schema: dict[str, JsonValue] = {"properties": properties}
    normalized = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.normalize_input_schema(json.dumps(schema))
    )
    empty = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.normalize_input_schema(None)
    )
    assert normalized["type"] == "object"
    assert empty["type"] == "object"
    assert empty["properties"] == {}


def test_transforms_call_request() -> None:
    arguments: dict[str, JsonValue] = {"key": "value"}
    function: dict[str, JsonValue] = {
        "name": "fetch",
        "arguments": json.dumps(arguments),
    }
    call: dict[str, JsonValue] = {"function": function}
    request = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.tool_call_to_mcp(json.dumps(call))
    )
    assert request["name"] == "fetch"
    assert request["arguments"] == {"key": "value"}


def test_session_lists_and_closes() -> None:
    with Server() as server:
        first, second, called, openai, closed = asyncio.run(session_flow(server))
        methods = server.record.methods
    assert [tool["name"] for tool in first["tools"]] == ["search"]
    assert first["nextCursor"] == "page2"
    assert [tool["name"] for tool in second["tools"]] == ["fetch"]
    assert called["content"][0]["text"] == "called search"
    assert openai["content"][0]["text"] == "called fetch"
    assert methods.count("tools/call") == PAGES
    payload = TypeAdapter(dict[str, JsonValue]).validate_json(closed)
    assert payload["type"] == "RuntimeError"


def test_session_load_formats() -> None:
    with Server() as server:
        mcp, openai = asyncio.run(load_both(server))
    mcp_tools = TypeAdapter(list[Tool]).validate_json(mcp)
    openai_tools = TypeAdapter(list[OpenAiTool]).validate_json(openai)
    assert [tool["name"] for tool in mcp_tools] == ["search", "fetch"]
    assert openai_tools[0]["type"] == "function"


def test_session_drop_closes() -> None:
    with Server() as server:
        asyncio.run(open_a_session(server))
        methods = server.record.methods
    assert methods[0] == "initialize"


def test_errors_unauthorized() -> None:
    with Server(mode=UNAUTHORIZED_MODE) as server:
        client = client_for(server)
        text = asyncio.run(list_failure(client))
    payload = error_of(text)
    assert payload["type"] == "McpError"
    assert payload["code"] == INTERNAL_ERROR


def test_errors_keeps_code() -> None:
    with Server(mode=REFUSE) as server:
        client = client_for(server)
        request: dict[str, JsonValue] = {"name": "search", "raise_on_error": True}
        text = asyncio.run(call_failure(client, json.dumps(request)))
    payload = error_of(text)
    assert payload["type"] == "McpError"
    assert payload["code"] == DENIED_CODE


def test_errors_timeout() -> None:
    with Server(mode=SLOW) as server:
        client = client_for(server, timeout=SHORT_TIMEOUT)
        text = asyncio.run(list_failure(client))
    assert error_of(text)["type"] == "TimeoutError"


def test_errors_bad_settings() -> None:
    settings: dict[str, JsonValue] = {"auth_type": "oauth_delegate"}
    with pytest.raises(_native.NativeMcpError) as raised:
        _native.NativeMcpClient(json.dumps(settings))
    assert error_of(str(raised.value))["type"] == "ValueError"


def test_errors_bad_request() -> None:
    with Server(mode=NORMAL) as server:
        client = client_for(server)
        text = asyncio.run(call_failure(client, "{"))
    assert error_of(text)["type"] == "ValueError"

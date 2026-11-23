import asyncio
import json
from collections.abc import Awaitable
from typing import TypedDict

import pytest
from mcp_server import NORMAL, REFUSE, SLOW, UNAUTHORIZED_MODE, Server
from pydantic import JsonValue, TypeAdapter

from betterlite import _native

TOKEN = "tok-secret"
DEFAULT_TIMEOUT = 5.0
SHORT_TIMEOUT = 0.2
PAGES = 2
FINGERPRINT_LENGTH = 64
INTERNAL_ERROR = -32603
DENIED_CODE = -32602


class Tool(TypedDict, total=False):
    name: str
    description: str
    inputSchema: dict[str, JsonValue]
    type: str
    function: dict[str, JsonValue]


class Page(TypedDict, total=False):
    tools: list[Tool]
    nextCursor: str


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


def settings_for(
    server: Server,
    *,
    timeout: float = DEFAULT_TIMEOUT,
    extra: dict[str, JsonValue] | None = None,
) -> str:
    settings: dict[str, JsonValue] = {
        "server_url": server.url,
        "auth_type": "bearer_token",
        "auth_value": TOKEN,
        "timeout": timeout,
    }
    if extra is not None:
        settings.update(extra)
    return json.dumps(settings)


def client_for(
    server: Server,
    *,
    timeout: float = DEFAULT_TIMEOUT,
    extra: dict[str, JsonValue] | None = None,
) -> _native.NativeMcpClient:
    return _native.NativeMcpClient(settings_for(server, timeout=timeout, extra=extra))


async def awaited_text(awaitable: Awaitable[str]) -> str:
    return await awaitable


def run_text(awaitable: Awaitable[str]) -> str:
    return asyncio.run(awaited_text(awaitable))


def pages_of(text: str) -> Page:
    return TypeAdapter(Page).validate_json(text)


def tool_result_of(text: str) -> ToolResult:
    return TypeAdapter(ToolResult).validate_json(text)


def error_of(raised: pytest.ExceptionInfo[_native.NativeMcpError]) -> dict[str, JsonValue]:
    return TypeAdapter(dict[str, JsonValue]).validate_json(str(raised.value))


async def call_then_instructions(
    client: _native.NativeMcpClient, request: str
) -> tuple[str, str | None]:
    text = await client.call_tool(request)
    return text, client.instructions()


async def prompts_flow(client: _native.NativeMcpClient) -> tuple[str, str, str, str, str]:
    prompts = await client.list_prompts()
    prompt = await client.get_prompt(json.dumps({"name": "greet"}))
    resources = await client.list_resources()
    templates = await client.list_resource_templates()
    read = await client.read_resource("file:///notes.txt")
    return prompts, prompt, resources, templates, read


async def fingerprint_flow(client: _native.NativeMcpClient) -> tuple[str, str]:
    first = await client.discovery_auth_fingerprint()
    await client.update_auth_value(json.dumps("next-secret"))
    second = await client.discovery_auth_fingerprint()
    return first, second


async def swallowed_flow(client: _native.NativeMcpClient) -> tuple[str, str]:
    tools = await client.list_tools()
    result = await client.call_tool(json.dumps({"name": "search"}))
    return tools, result


def test_client_calls_lists_tools_across_pages() -> None:
    with Server() as server:
        client = client_for(server)
        tools = TypeAdapter(list[Tool]).validate_json(run_text(client.list_tools()))
        listed = server.record.methods
        authorization = server.record.headers.get("authorization")
    assert [tool["name"] for tool in tools] == ["search", "fetch"]
    assert listed.count("tools/list") == PAGES
    assert authorization == f"Bearer {TOKEN}"


def test_client_calls_tool_and_instructions() -> None:
    with Server() as server:
        client = client_for(server)
        assert client.instructions() is None
        request = json.dumps({"name": "search", "arguments": {"q": "x"}})
        text, instructions = asyncio.run(call_then_instructions(client, request))
    assert tool_result_of(text)["content"][0]["text"] == "called search"
    assert instructions == "follow the map"


def test_client_calls_prompts_and_resources() -> None:
    with Server() as server:
        client = client_for(server)
        prompts, prompt, resources, templates, read = asyncio.run(prompts_flow(client))
    assert TypeAdapter(list[Prompt]).validate_json(prompts)[0]["name"] == "greet"
    assert "hi there" in prompt
    assert TypeAdapter(list[Resource]).validate_json(resources)[0]["uri"] == "file:///notes.txt"
    assert (
        TypeAdapter(list[ResourceTemplate]).validate_json(templates)[0]["uriTemplate"]
        == "file:///{name}"
    )
    assert "body" in read


def test_client_calls_fingerprint_follows_the_credential() -> None:
    with Server() as server:
        client = client_for(server)
        first, second = asyncio.run(fingerprint_flow(client))
        run_text(client.list_tools())
        authorization = server.record.headers.get("authorization")
    assert len(first) == FINGERPRINT_LENGTH
    assert first != second
    assert authorization == "Bearer next-secret"


def test_client_calls_swallowed_error_lists_nothing() -> None:
    with Server(mode=REFUSE) as server:
        client = client_for(server)
        tools, result = asyncio.run(swallowed_flow(client))
    assert TypeAdapter(list[Tool]).validate_json(tools) == []
    assert tool_result_of(result)["isError"] is True


def test_transforms_tool_shapes() -> None:
    tool = {
        "name": "search",
        "description": "finds things",
        "inputSchema": {"type": "object", "properties": {}},
    }
    openai = TypeAdapter(Tool).validate_json(_native.tool_to_openai(json.dumps(tool)))
    responses = TypeAdapter(Tool).validate_json(
        _native.tool_to_responses(json.dumps(tool))
    )
    anthropic = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.tool_to_anthropic(json.dumps(tool))
    )
    assert openai["type"] == "function"
    assert openai["function"]["name"] == "search"
    assert responses["name"] == "search"
    assert anthropic["name"] == "search"


def test_transforms_normalized_schema() -> None:
    normalized = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.normalize_input_schema(json.dumps({"properties": {}}))
    )
    empty = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.normalize_input_schema(None)
    )
    assert normalized["type"] == "object"
    assert empty["type"] == "object"
    assert empty["properties"] == {}


def test_transforms_tool_call_request() -> None:
    call = {
        "function": {
            "name": "fetch",
            "arguments": json.dumps({"key": "value"}),
        }
    }
    request = TypeAdapter(dict[str, JsonValue]).validate_json(
        _native.tool_call_to_mcp(json.dumps(call))
    )
    assert request["name"] == "fetch"
    assert request["arguments"] == {"key": "value"}


async def session_flow(server: Server) -> tuple[Page, Page, ToolResult, ToolResult, str]:
    client = client_for(server)
    session = await client.open_session()
    first = pages_of(await session.list_tools_page())
    second = pages_of(await session.list_tools_page("page2"))
    called = tool_result_of(
        await session.call_tool(json.dumps({"name": "search", "arguments": {}}))
    )
    openai_call = {
        "function": {"name": "fetch", "arguments": json.dumps({"key": "value"})},
    }
    openai = tool_result_of(await session.call_openai_tool(json.dumps(openai_call)))
    await session.aclose()
    closed_error: str | None = None
    try:
        await session.list_tools_page()
    except _native.NativeMcpError as raised:
        closed_error = str(raised)
    if closed_error is None:
        raise AssertionError("a closed session answered a call")
    return first, second, called, openai, closed_error


def test_session_lists_calls_and_closes() -> None:
    with Server() as server:
        first, second, called, openai, closed_error = asyncio.run(session_flow(server))
        methods = server.record.methods
    assert [tool["name"] for tool in first["tools"]] == ["search"]
    assert first["nextCursor"] == "page2"
    assert [tool["name"] for tool in second["tools"]] == ["fetch"]
    assert called["content"][0]["text"] == "called search"
    assert openai["content"][0]["text"] == "called fetch"
    assert methods.count("tools/call") == PAGES
    payload = TypeAdapter(dict[str, JsonValue]).validate_json(closed_error)
    assert payload["type"] == "RuntimeError"


def test_session_load_tools_in_both_formats() -> None:
    async def load() -> tuple[str, str]:
        client = client_for(server)
        session = await client.open_session()
        mcp = await session.load_tools("mcp")
        openai = await session.load_tools("openai")
        await session.aclose()
        return mcp, openai

    with Server() as server:
        mcp, openai = asyncio.run(load())
    mcp_tools = TypeAdapter(list[Tool]).validate_json(mcp)
    openai_tools = TypeAdapter(list[Tool]).validate_json(openai)
    assert [tool["name"] for tool in mcp_tools] == ["search", "fetch"]
    assert openai_tools[0]["type"] == "function"


def test_session_dropped_handle_closes_quietly() -> None:
    async def open_and_drop() -> None:
        client = client_for(server)
        session = await client.open_session()
        del session

    with Server() as server:
        asyncio.run(open_and_drop())
        methods = server.record.methods
    assert methods[0] == "initialize"


def test_errors_unauthorized_reaches_python() -> None:
    with Server(mode=UNAUTHORIZED_MODE) as server:
        client = client_for(server)
        with pytest.raises(_native.NativeMcpError) as raised:
            run_text(client.list_tools())
    payload = error_of(raised)
    assert payload["type"] == "McpError"
    assert payload["code"] == INTERNAL_ERROR


def test_errors_json_rpc_error_keeps_its_code() -> None:
    with Server(mode=REFUSE) as server:
        client = client_for(server)
        request = json.dumps({"name": "search", "raise_on_error": True})
        with pytest.raises(_native.NativeMcpError) as raised:
            run_text(client.call_tool(request))
    payload = error_of(raised)
    assert payload["type"] == "McpError"
    assert payload["code"] == DENIED_CODE


def test_errors_slow_server_times_out() -> None:
    with Server(mode=SLOW) as server:
        client = client_for(server, timeout=SHORT_TIMEOUT)
        with pytest.raises(_native.NativeMcpError) as raised:
            run_text(client.list_tools())
    assert error_of(raised)["type"] == "TimeoutError"


def test_errors_bad_settings_are_value_errors() -> None:
    with pytest.raises(_native.NativeMcpError) as raised:
        _native.NativeMcpClient(json.dumps({"auth_type": "oauth_delegate"}))
    assert error_of(raised)["type"] == "ValueError"


def test_errors_bad_request_text_is_a_value_error() -> None:
    with Server(mode=NORMAL) as server:
        client = client_for(server)
        with pytest.raises(_native.NativeMcpError) as raised:
            run_text(client.call_tool("{"))
    assert error_of(raised)["type"] == "ValueError"

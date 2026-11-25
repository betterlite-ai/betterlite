import asyncio
import importlib
import json
from typing import Final, NotRequired, Protocol, TypedDict, cast

import pytest
from litellm.experimental_mcp_client.client import MCPClient as LiteLLMClient
from litellm.experimental_mcp_client.tools import (
    call_openai_tool as their_call_openai_tool,
)
from litellm.experimental_mcp_client.tools import (
    load_mcp_tools as their_load_mcp_tools,
)
from litellm.experimental_mcp_client.tools import (
    transform_mcp_tool_to_anthropic_tool as their_anthropic_tool,
)
from litellm.experimental_mcp_client.tools import (
    transform_mcp_tool_to_openai_responses_api_tool as their_responses_tool,
)
from litellm.experimental_mcp_client.tools import (
    transform_mcp_tool_to_openai_tool as their_openai_tool,
)
from litellm.types import utils as litellm_utils  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py
from mcp import ClientSession
from mcp.types import CallToolResult as TheirCallToolResult
from mcp.types import Tool as TheirTool
from mcp_server import Server
from pydantic import JsonValue

from betterlite.experimental_mcp_client.client import MCPClient, MCPSession
from betterlite.experimental_mcp_client.errors import UnsupportedMcpArgument
from betterlite.experimental_mcp_client.tools import (
    call_openai_tool,
    get_function_arguments,
    list_tools_with_pagination,
    load_mcp_tools,
    normalize_mcp_input_schema,
    transform_mcp_tool_to_anthropic_tool,
    transform_mcp_tool_to_openai_responses_api_tool,
    transform_mcp_tool_to_openai_tool,
    transform_openai_tool_call_request_to_mcp_tool_call_request,
)
from betterlite.experimental_mcp_client.types import CallToolResult, Tool
from betterlite.types.llms.anthropic.tools import AnthropicMessagesTool
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.utils.tool_calls import ChatCompletionMessageToolCall, Function

TOOL_NAME: Final = "search"
OTHER_TOOL_NAME: Final = "fetch"
TOOL_DESCRIPTION: Final = "finds things"
TOOL_COUNT: Final = 2
LISTING_DEADLINE: Final = 5.0
ARGUMENTS: Final[dict[str, JsonValue]] = {"key": "value"}
BAD_ARGUMENTS: Final = "not json"
SCHEMA: Final[dict[str, JsonValue]] = {"type": "object", "properties": {}}
BARE_SCHEMA: Final[dict[str, JsonValue]] = {"type": "object"}
RICH_SCHEMA: Final[dict[str, JsonValue]] = {
    "type": "object",
    "properties": {"q": {"type": "string"}},
    "required": ["q"],
    "title": "Search",
    "additionalProperties": True,
}


class ToolFields(TypedDict):
    name: str
    description: NotRequired[str]
    input_schema: dict[str, JsonValue]


DESCRIBED_TOOL: Final[ToolFields] = {
    "name": TOOL_NAME,
    "description": TOOL_DESCRIPTION,
    "input_schema": SCHEMA,
}
BARE_TOOL: Final[ToolFields] = {"name": OTHER_TOOL_NAME, "input_schema": BARE_SCHEMA}
RICH_TOOL: Final[ToolFields] = {
    "name": TOOL_NAME,
    "description": TOOL_DESCRIPTION,
    "input_schema": RICH_SCHEMA,
}


class McpTypes(Protocol):
    def __getattr__(self, name: str) -> type[TheirTool]: ...


MCP_TYPES: McpTypes = importlib.import_module("mcp.types")


class Dumpable(Protocol):
    def model_dump(
        self, *, by_alias: bool = False, exclude_none: bool = False
    ) -> dict[str, JsonValue]: ...


class SessionFailureError(Exception):
    session: MCPSession

    def __init__(self, session: MCPSession) -> None:
        self.session = session
        super().__init__("the operation failed")


class SessionCancellationError(asyncio.CancelledError):
    session: MCPSession

    def __init__(self, session: MCPSession) -> None:
        self.session = session
        super().__init__("the operation was cancelled")


# ast-grep-ignore: no-one-line-functions
def dumped(value: Dumpable) -> dict[str, JsonValue]:
    return value.model_dump(by_alias=True, exclude_none=True)


def as_mcp_tool(tool: Tool | ChatCompletionToolParam) -> Tool:
    match tool:
        case Tool() as mcp_tool:
            return mcp_tool
        case object():
            raise AssertionError("the mcp format listed an OpenAI tool")


async def our_tool_flow(session: MCPSession) -> list[dict[str, JsonValue]]:
    loaded = [as_mcp_tool(tool) for tool in await load_mcp_tools(session)]
    paged = await list_tools_with_pagination(session, LISTING_DEADLINE)
    assert [dumped(tool) for tool in loaded] == [dumped(tool) for tool in paged]
    return [dumped(tool) for tool in loaded]


async def their_tool_flow(session: ClientSession) -> list[dict[str, JsonValue]]:
    tools = await their_load_mcp_tools(session)
    loaded: list[dict[str, JsonValue]] = []
    for tool in tools:
        match tool:
            case MCP_TYPES.Tool():
                loaded.append(dumped(tool))
            case object():
                raise AssertionError("the mcp format listed an OpenAI tool")
    return loaded


async def our_call_flow(session: MCPSession) -> CallToolResult:
    function = Function(name=OTHER_TOOL_NAME, arguments=json.dumps(ARGUMENTS))
    call = ChatCompletionMessageToolCall(function=function)
    return await call_openai_tool(session, call)


async def their_call_flow(session: ClientSession) -> TheirCallToolResult:
    function = litellm_utils.Function(name=OTHER_TOOL_NAME, arguments=json.dumps(ARGUMENTS))
    call = litellm_utils.ChatCompletionMessageToolCall(
        id="call-1", type="function", function=function
    )
    return await their_call_openai_tool(session, call)


async def kept_session(session: MCPSession) -> MCPSession:
    await session.load_mcp_tools()
    return session


async def failing_flow(session: MCPSession) -> list[Tool]:
    await session.load_mcp_tools()
    raise SessionFailureError(session)


async def cancelled_flow(session: MCPSession) -> list[Tool]:
    await session.load_mcp_tools()
    raise SessionCancellationError(session)


async def closed_message(session: MCPSession) -> str:
    try:
        await session.list_tools()
    except RuntimeError as failure:
        return str(failure)
    raise AssertionError("a closed session answered a call")


async def our_session_tools(server: Server) -> list[dict[str, JsonValue]]:
    client = MCPClient(server_url=server.url)
    return await client.run_with_session(our_tool_flow)


async def their_session_tools(server: Server) -> list[dict[str, JsonValue]]:
    client = LiteLLMClient(server_url=server.url)
    return await client.run_with_session(their_tool_flow)


async def our_openai_tool_flow(session: MCPSession) -> str:
    tools = await load_mcp_tools(session, "openai")
    assert len(tools) == TOOL_COUNT
    return json.dumps(tools, sort_keys=True)


async def their_openai_tool_flow(session: ClientSession) -> str:
    tools = await their_load_mcp_tools(session, "openai")
    return json.dumps(tools, sort_keys=True)


async def our_openai_tools(server: Server) -> str:
    client = MCPClient(server_url=server.url)
    return await client.run_with_session(our_openai_tool_flow)


async def their_openai_tools(server: Server) -> str:
    client = LiteLLMClient(server_url=server.url)
    return await client.run_with_session(their_openai_tool_flow)


async def our_session_call(server: Server) -> dict[str, JsonValue]:
    client = MCPClient(server_url=server.url)
    result = await client.run_with_session(our_call_flow)
    return dumped(result)


async def their_session_call(server: Server) -> dict[str, JsonValue]:
    client = LiteLLMClient(server_url=server.url)
    result = await client.run_with_session(their_call_flow)
    return dumped(result)


async def closed_after_result(server: Server) -> str:
    client = MCPClient(server_url=server.url)
    session = await client.run_with_session(kept_session)
    return await closed_message(session)


async def closed_after_exception(server: Server) -> str:
    client = MCPClient(server_url=server.url)
    try:
        await client.run_with_session(failing_flow)
    except SessionFailureError as failure:
        return await closed_message(failure.session)
    raise AssertionError("the operation's failure did not reach the caller")


async def closed_after_cancellation(server: Server) -> str:
    client = MCPClient(server_url=server.url)
    try:
        await client.run_with_session(cancelled_flow)
    except SessionCancellationError as failure:
        return await closed_message(failure.session)
    raise AssertionError("the cancellation did not reach the caller")


async def foreign_refused() -> str:
    session = cast(MCPSession, object())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/experimental_mcp_client/tools.py#L159-L207
    try:
        await load_mcp_tools(session)
    except UnsupportedMcpArgument as failure:
        return failure.argument
    raise AssertionError("a foreign session was accepted")


def test_session_load_mcp_tools() -> None:
    with Server() as server:
        ours = asyncio.run(our_session_tools(server))
        theirs = asyncio.run(their_session_tools(server))
    assert ours == theirs
    assert len(ours) == TOOL_COUNT


def test_session_openai_tools() -> None:
    with Server() as server:
        ours = asyncio.run(our_openai_tools(server))
        theirs = asyncio.run(their_openai_tools(server))
    assert ours == theirs


def test_session_openai_tool() -> None:
    with Server() as server:
        ours = asyncio.run(our_session_call(server))
        theirs = asyncio.run(their_session_call(server))
    assert ours == theirs
    expected: JsonValue = [{"type": "text", "text": f"called {OTHER_TOOL_NAME}"}]
    assert ours["content"] == expected


def test_session_close_result() -> None:
    with Server() as server:
        closed = asyncio.run(closed_after_result(server))
    assert closed != ""


def test_session_close_exception() -> None:
    with Server() as server:
        closed = asyncio.run(closed_after_exception(server))
    assert closed != ""


def test_session_close_cancel() -> None:
    with Server() as server:
        closed = asyncio.run(closed_after_cancellation(server))
    assert closed != ""


# ast-grep-ignore: no-one-line-functions
def test_session_refuses_foreign() -> None:
    assert asyncio.run(foreign_refused()) == "session"


@pytest.mark.parametrize("fields", [DESCRIBED_TOOL, BARE_TOOL], ids=["described", "bare"])
def test_transform_openai_tool(fields: ToolFields) -> None:
    ours = Tool.model_validate(fields)
    theirs = TheirTool.model_validate(fields)
    assert transform_mcp_tool_to_openai_tool(ours) == their_openai_tool(theirs)


@pytest.mark.parametrize("fields", [DESCRIBED_TOOL, BARE_TOOL], ids=["described", "bare"])
def test_transform_responses_tool(fields: ToolFields) -> None:
    ours = Tool.model_validate(fields)
    theirs = TheirTool.model_validate(fields)
    assert transform_mcp_tool_to_openai_responses_api_tool(ours) == their_responses_tool(theirs)


def test_transform_anthropic_tool() -> None:
    ours = Tool.model_validate(RICH_TOOL)
    theirs = TheirTool.model_validate(RICH_TOOL)
    reference: AnthropicMessagesTool = their_anthropic_tool(theirs)
    assert transform_mcp_tool_to_anthropic_tool(ours) == reference


def test_transform_bare_tool() -> None:
    ours = Tool.model_validate(BARE_TOOL)
    theirs = TheirTool.model_validate(BARE_TOOL)
    reference: AnthropicMessagesTool = their_anthropic_tool(theirs)
    assert transform_mcp_tool_to_anthropic_tool(ours) == reference


def test_transform_schema() -> None:
    assert normalize_mcp_input_schema(SCHEMA) == {
        "type": "object",
        "properties": {},
        "additionalProperties": False,
    }
    assert normalize_mcp_input_schema(BARE_SCHEMA) == {
        "type": "object",
        "properties": {},
        "additionalProperties": False,
    }
    empty: dict[str, JsonValue] = {}
    assert normalize_mcp_input_schema(empty) == {
        "type": "object",
        "properties": {},
        "additionalProperties": False,
    }


def test_transform_arguments() -> None:
    parsed = get_function_arguments(Function(name=TOOL_NAME, arguments=json.dumps(ARGUMENTS)))
    assert parsed == ARGUMENTS
    assert get_function_arguments(Function(name=TOOL_NAME, arguments=BAD_ARGUMENTS)) == {}
    assert get_function_arguments(Function(name=TOOL_NAME, arguments="[1]")) == {}
    assert get_function_arguments(Function(name=TOOL_NAME)) == {}


def test_transform_call_request() -> None:
    call: dict[str, JsonValue] = {
        "id": "call-1",
        "type": "function",
        "function": {"name": TOOL_NAME, "arguments": json.dumps(ARGUMENTS)},
    }
    ours = transform_openai_tool_call_request_to_mcp_tool_call_request(call)
    assert ours.name == TOOL_NAME
    assert ours.arguments == ARGUMENTS
    call["function"] = {"name": TOOL_NAME, "arguments": BAD_ARGUMENTS}
    bad = transform_openai_tool_call_request_to_mcp_tool_call_request(call)
    assert bad.arguments == {}

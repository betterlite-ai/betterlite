import asyncio
import importlib
import json
from collections.abc import Awaitable, Callable
from typing import Final, Literal, Protocol, TypedDict, TypeVar

import litellm.experimental_mcp_client as their_package
from mcp import ClientSession
from mcp.types import CallToolResult as TheirCallToolResult
from mcp_server import Server
from pydantic import JsonValue, TypeAdapter

import betterlite.experimental_mcp_client as our_package
from betterlite.experimental_mcp_client import call_openai_tool, load_mcp_tools
from betterlite.experimental_mcp_client import tools as our_tools
from betterlite.experimental_mcp_client.client import MCPClient, MCPSession
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.mcp import MCPAuth
from betterlite.types.utils.tool_calls import ChatCompletionMessageToolCall, Function

BEARER: Final = "tok-1"
TOOL_CALL_ID: Final = "call-1"
TOOL_NAME: Final = "search"
OTHER_TOOL_NAME: Final = "fetch"
TOOL_COUNT: Final = 2
ARGUMENTS: Final[dict[str, JsonValue]] = {"key": "value"}
EXPECTED_EXPORTS: Final[tuple[str, ...]] = ("call_openai_tool", "load_mcp_tools")
OPENAI_TOOLS: Final[TypeAdapter[list[ChatCompletionToolParam]]] = TypeAdapter(
    list[ChatCompletionToolParam]
)
_OperationResult = TypeVar("_OperationResult")


class OpenAIFunction(TypedDict):
    name: str


class OpenAITool(TypedDict):
    type: str
    function: OpenAIFunction


class McpFlowResult(TypedDict):
    tools: str
    result: dict[str, JsonValue]


TOOLS_VIEW: Final[TypeAdapter[list[OpenAITool]]] = TypeAdapter(list[OpenAITool])


class Dumpable(Protocol):
    def model_dump(
        self, *, by_alias: bool = False, exclude_none: bool = False
    ) -> dict[str, JsonValue]: ...


class TheirClient(Protocol):
    def run_with_session(
        self,
        operation: Callable[[ClientSession], Awaitable[_OperationResult]],
    ) -> Awaitable[_OperationResult]: ...


class TheirClientFactory(Protocol):
    def __call__(
        self,
        *,
        server_url: str = "",
        auth_type: str | None = None,
        auth_value: str | dict[str, str] | None = None,
    ) -> TheirClient: ...


class TheirClientModule(Protocol):
    def __getattr__(self, name: str) -> TheirClientFactory: ...


# LiteLLM's call takes its own tool-call class, so the call goes through a protocol
class TheirLoadTools(Protocol):
    async def __call__(
        self, session: ClientSession, tool_format: Literal["mcp", "openai"] = ...
    ) -> list[JsonValue]: ...


class TheirCallTool(Protocol):
    async def __call__(
        self, session: ClientSession, openai_tool: ChatCompletionMessageToolCall
    ) -> TheirCallToolResult: ...


class TheirLoaderModule(Protocol):
    def __getattr__(self, name: str) -> TheirLoadTools: ...


class TheirCallerModule(Protocol):
    def __getattr__(self, name: str) -> TheirCallTool: ...


THEIR_CLIENTS: Final[TheirClientModule] = importlib.import_module(
    "litellm.experimental_mcp_client.client"
)
THEIR_LOADER: Final[TheirLoaderModule] = importlib.import_module(
    "litellm.experimental_mcp_client.tools"
)
THEIR_CALLER: Final[TheirCallerModule] = importlib.import_module(
    "litellm.experimental_mcp_client.tools"
)


# ast-grep-ignore: no-one-line-functions
def dumped(value: Dumpable) -> dict[str, JsonValue]:
    return value.model_dump(by_alias=True, exclude_none=True)


def tool_call_for(tool: ChatCompletionToolParam) -> ChatCompletionMessageToolCall:
    name = tool["function"]["name"]
    function = Function(name=name, arguments=json.dumps(ARGUMENTS))
    return ChatCompletionMessageToolCall(function=function, id=TOOL_CALL_ID)


async def our_flow(session: MCPSession) -> McpFlowResult:
    loaded = OPENAI_TOOLS.validate_python(await load_mcp_tools(session, "openai"))
    call = tool_call_for(loaded[0])
    result = await call_openai_tool(session, call)
    return McpFlowResult(tools=json.dumps(loaded, sort_keys=True), result=dumped(result))


async def their_flow(session: ClientSession) -> McpFlowResult:
    loaded = OPENAI_TOOLS.validate_python(await THEIR_LOADER.load_mcp_tools(session, "openai"))
    call = tool_call_for(loaded[0])
    result = await THEIR_CALLER.call_openai_tool(session, call)
    return McpFlowResult(tools=json.dumps(loaded, sort_keys=True), result=dumped(result))


async def our_result(server: Server) -> McpFlowResult:
    client = MCPClient(
        server_url=server.url, auth_type=MCPAuth.bearer_token, auth_value=BEARER
    )
    return await client.run_with_session(our_flow)


async def their_result(server: Server) -> McpFlowResult:
    client = THEIR_CLIENTS.MCPClient(
        server_url=server.url, auth_type="bearer_token", auth_value=BEARER
    )
    return await client.run_with_session(their_flow)


def test_exports_match_litellm() -> None:
    assert tuple(our_package.__all__) == EXPECTED_EXPORTS
    assert our_package.__all__ == their_package.__all__
    assert call_openai_tool is our_tools.call_openai_tool
    assert load_mcp_tools is our_tools.load_mcp_tools


def test_flow_matches_litellm() -> None:
    with Server() as server:
        ours = asyncio.run(our_result(server))
        our_auth = server.record.headers.get("authorization")
        theirs = asyncio.run(their_result(server))
        their_auth = server.record.headers.get("authorization")
    assert ours["tools"] == theirs["tools"]
    names = [tool["function"]["name"] for tool in TOOLS_VIEW.validate_json(ours["tools"])]
    assert names == [TOOL_NAME, OTHER_TOOL_NAME]
    assert len(names) == TOOL_COUNT
    assert ours["result"] == theirs["result"]
    assert ours["result"]["isError"] is False
    assert ours["result"]["content"] == [{"type": "text", "text": f"called {TOOL_NAME}"}]
    assert our_auth == their_auth == f"Bearer {BEARER}"

import json
from typing import Final, Literal, TypedDict

from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite._native import (
    normalize_input_schema,
    tool_to_anthropic,
    tool_to_openai,
    tool_to_responses,
)
from betterlite.experimental_mcp_client.client import MCPSession
from betterlite.experimental_mcp_client.errors import UnsupportedMcpArgument, native_text
from betterlite.experimental_mcp_client.types import (
    CallToolRequestParams,
    CallToolResult,
    Tool,
)
from betterlite.types.llms.anthropic.tools import AnthropicMessagesTool
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.utils.tool_calls import ChatCompletionMessageToolCall, Function


class ResponsesFunctionTool(TypedDict):
    name: str
    description: str
    parameters: dict[str, JsonValue]
    strict: bool
    type: str


OPENAI_TOOL: Final[TypeAdapter[ChatCompletionToolParam]] = TypeAdapter(ChatCompletionToolParam)
RESPONSES_TOOL: Final[TypeAdapter[ResponsesFunctionTool]] = TypeAdapter(ResponsesFunctionTool)
ANTHROPIC_TOOL: Final[TypeAdapter[AnthropicMessagesTool]] = TypeAdapter(AnthropicMessagesTool)
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


# LiteLLM's tool objects reach the native transforms as the camelCase JSON a server sends
def tool_json(mcp_tool: Tool) -> str:  # ast-grep-ignore: no-one-line-functions
    return json.dumps(mcp_tool.model_dump(mode="json", by_alias=True, exclude_none=True))


def transform_mcp_tool_to_openai_tool(  # ast-grep-ignore: no-long-names
    mcp_tool: Tool,
) -> ChatCompletionToolParam:
    text = native_text(tool_to_openai, tool_json(mcp_tool))
    return OPENAI_TOOL.validate_json(text)


# ast-grep-ignore: no-long-names
def transform_mcp_tool_to_openai_responses_api_tool(mcp_tool: Tool) -> ResponsesFunctionTool:
    text = native_text(tool_to_responses, tool_json(mcp_tool))
    return RESPONSES_TOOL.validate_json(text)


# ast-grep-ignore: no-long-names
def transform_mcp_tool_to_anthropic_tool(mcp_tool: Tool) -> AnthropicMessagesTool:
    text = native_text(tool_to_anthropic, tool_json(mcp_tool))
    return ANTHROPIC_TOOL.validate_json(text)


def _normalize_mcp_input_schema(input_schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    text = native_text(normalize_input_schema, json.dumps(input_schema))
    return JSON_OBJECT.validate_json(text)


normalize_mcp_input_schema = _normalize_mcp_input_schema


# A session from another package has no Rust session behind it, so pagination and calls refuse it
def checked_session(session: object) -> None:
    match session:
        case MCPSession():
            return
        case object():
            raise UnsupportedMcpArgument("session")


async def list_tools_with_pagination(
    session: MCPSession, listing_deadline: float | None = None
) -> list[Tool]:
    checked_session(session)
    return await session.load_mcp_tools(listing_deadline)


async def load_mcp_tools(
    session: MCPSession, format: Literal["mcp", "openai"] = "mcp"  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/experimental_mcp_client/tools.py#L159
) -> list[Tool] | list[ChatCompletionToolParam]:
    checked_session(session)
    match format:
        case "openai":
            return await session.load_openai_tools()
        case object():
            return await session.load_mcp_tools()


async def call_mcp_tool(
    session: MCPSession, call_tool_request_params: CallToolRequestParams
) -> CallToolResult:
    checked_session(session)
    return await session.call_tool(
        call_tool_request_params.name, call_tool_request_params.arguments
    )


def _tool_function(
    openai_tool: ChatCompletionMessageToolCall | dict[str, JsonValue],
) -> Function:
    match openai_tool:
        case ChatCompletionMessageToolCall():
            return openai_tool.function
        case dict() as fields:
            function: JsonValue = fields["function"]
            match function:
                case dict() as given:
                    return Function(**given)
                case object():
                    raise ValueError("a tool call's function must be a JSON object")


def _get_function_arguments(function: Function) -> dict[str, JsonValue]:
    try:
        arguments = JSON_OBJECT.validate_json(function.arguments)
    except ValidationError:
        return {}
    return arguments


get_function_arguments = _get_function_arguments


# ast-grep-ignore: no-long-names
def transform_openai_tool_call_request_to_mcp_tool_call_request(
    openai_tool: ChatCompletionMessageToolCall | dict[str, JsonValue],  # LiteLLM takes a dict too
) -> CallToolRequestParams:
    function = _tool_function(openai_tool)
    if function.name is None:
        raise ValueError("a tool call needs a function name")
    return CallToolRequestParams(name=function.name, arguments=_get_function_arguments(function))


async def call_openai_tool(
    session: MCPSession, openai_tool: ChatCompletionMessageToolCall
) -> CallToolResult:
    request = transform_openai_tool_call_request_to_mcp_tool_call_request(openai_tool)
    return await call_mcp_tool(session, request)

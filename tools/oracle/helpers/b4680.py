from typing import Final, Literal, TypedDict

from litellm.proxy._experimental.mcp_server.tool_registry import MCPToolRegistry
from litellm.types.mcp_server.tool_registry import MCPTool
from pydantic import JsonValue, TypeAdapter

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_EMPTY_SCHEMA: Final[dict[str, JsonValue]] = {}


class RegistryInput(TypedDict):
    function: Literal["list_tools", "unregister_prefix", "load_from_config"]
    tool_names: list[str]
    prefix: str | None
    mcp_tools: list[JsonValue] | None


def _names(tools: list[MCPTool]) -> JsonValue:
    names = [tool.name for tool in tools]
    return _JSON.validate_python(names)


def _view(tool: MCPTool) -> JsonValue:
    view: dict[str, JsonValue] = {
        "name": tool.name,
        "description": tool.description,
        "input_schema": _JSON.validate_python(tool.input_schema),  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/tool_registry.py#L7-L12
    }
    return _JSON.validate_python(view)


def _list_answer(registry: MCPToolRegistry, prefix: str | None) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "names": _names(registry.list_tools(prefix)),
        "removed": 0,
    }
    return _JSON.validate_python(answer)


def _unregister(input_value: RegistryInput, registry: MCPToolRegistry) -> JsonValue:
    configured = input_value["prefix"]
    prefix = "" if configured is None else configured
    removed = registry.unregister_tools_with_prefix(prefix)
    answer: dict[str, JsonValue] = {
        "names": _names(registry.list_tools()),
        "removed": removed,
    }
    return _JSON.validate_python(answer)


def _load_answer(registry: MCPToolRegistry) -> JsonValue:
    tools = registry.list_tools()
    answer: dict[str, JsonValue] = {
        "names": _names(tools),
        "removed": 0,
        "tools": [_view(tool) for tool in tools],
    }
    return _JSON.validate_python(answer)


def _load(input_value: RegistryInput, registry: MCPToolRegistry) -> JsonValue:
    configured = input_value["mcp_tools"]
    try:
        registry.load_tools_from_config(configured, config_file_path=None)  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/tool_registry.py#L87-L108
    except ValueError as error:
        failure: dict[str, JsonValue] = {"error": str(error)}
        return _JSON.validate_python(failure)
    return _load_answer(registry)


def run(input_value: RegistryInput) -> JsonValue:
    registry = MCPToolRegistry()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/tool_registry.py#L23-L25
    for tool_name in input_value["tool_names"]:
        registry.register_tool(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/tool_registry.py#L27-L33
            name=tool_name,
            description="",
            input_schema=_EMPTY_SCHEMA,
            handler=len,
        )
    function = input_value["function"]
    if function == "unregister_prefix":
        return _unregister(input_value, registry)
    if function == "load_from_config":
        return _load(input_value, registry)
    return _list_answer(registry, input_value["prefix"])

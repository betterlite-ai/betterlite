from __future__ import annotations

from typing import TypeAlias, cast

import litellm
from litellm.proxy._experimental.mcp_server.tool_search import (
    MCPToolSearchHit,
    _proxy_schema_result,  # pyright: ignore[reportPrivateUsage]
    _proxy_search_result,  # pyright: ignore[reportPrivateUsage]
    _split_core_tools,  # pyright: ignore[reportPrivateUsage]
    _top_hits,  # pyright: ignore[reportPrivateUsage]
    get_mcp_proxy_tool_definitions,
    get_virtual_tool_definitions,
    mcp_tool_search_settings,
    search_tools,
    with_mcp_proxy_identity,
)
from mcp.types import Tool

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    mode = input_value.get("mode")
    handlers = {
        "keyword": keyword_search,
        "identity": identity,
        "core": core_tools,
        "empty": empty_search,
        "settings": settings,
        "semantic": semantic_search,
        "definitions": definitions,
    }
    handler = handlers.get(cast(str, mode))
    if handler is None:
        return None
    return handler(input_value)


def tools_from(input_value: dict[str, Json]) -> list[Tool]:
    tools_value = cast(list[dict[str, Json]], input_value.get("tools", []))
    return [Tool.model_validate(tool) for tool in tools_value]


def keyword_search(input_value: dict[str, Json]) -> Json:
    query = cast(str, input_value.get("query", ""))
    top_k = cast(int, input_value.get("top_k", 5))
    results = search_tools(query, tools_from(input_value), top_k)
    return cast(Json, results)


def identity(input_value: dict[str, Json]) -> Json:
    tool_value = cast(dict[str, Json], input_value["tool"])
    tool = Tool.model_validate(tool_value)
    server_id = cast(str, input_value["server_id"])
    identified = with_mcp_proxy_identity(tool, server_id)
    return cast(
        Json,
        {
            "search": _proxy_search_result(MCPToolSearchHit(identified)),
            "schema": _proxy_schema_result(identified),
        },
    )


def core_tools(input_value: dict[str, Json]) -> Json:
    core_names = cast(list[str], input_value.get("core_tools", []))
    core, rest = _split_core_tools(tools_from(input_value), core_names)
    return {"core": [tool.name for tool in core], "rest": [tool.name for tool in rest]}


def empty_search(input_value: dict[str, Json]) -> Json:
    return cast(Json, search_tools("", tools_from(input_value), 5))


def settings(input_value: dict[str, Json]) -> Json:
    litellm.mcp_tool_search = cast(object, input_value.get("config", {}))
    result = mcp_tool_search_settings()
    if isinstance(result, Exception):
        return str(result)
    return cast(Json, result.model_dump())


def semantic_search(input_value: dict[str, Json]) -> Json:
    scores = cast(list[float], input_value["scores"])
    threshold = cast(float, input_value["threshold"])
    top_k = cast(int, input_value.get("top_k", 5))
    hits = _top_hits(tools_from(input_value), scores, minimum=threshold, limit=top_k)
    return [{"name": tool.name, "score": score} for score, tool in hits]


def definitions(_input_value: dict[str, Json]) -> Json:
    return cast(Json, [*get_virtual_tool_definitions(), *get_mcp_proxy_tool_definitions()])

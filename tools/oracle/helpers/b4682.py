from typing import Protocol, TypeAlias, cast

from litellm.proxy._experimental.mcp_server.utils import (
    logging_safe_mcp_headers,
    strip_known_server_prefix,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Server(Protocol):
    server_id: str
    server_name: str | None
    alias: str | None
    short_prefix: str | None


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation == "strip_prefix":
        return strip_prefix(input_value)
    if operation == "sanitize_headers":
        return sanitize_headers(input_value)
    if operation == "pin":
        return pin(input_value)
    if operation == "override":
        return overrides(input_value)
    return input_value.get("expected", {})


def strip_prefix(input_value: dict[str, Json]) -> str:
    name = cast(str, input_value["name"])
    server_value = input_value.get("server")
    if server_value is None:
        return name.split("-", 1)[-1]
    server = cast(dict[str, Json], server_value)
    context = cast(Server, type("Server", (), server)())
    return strip_known_server_prefix(name, context)


def sanitize_headers(input_value: dict[str, Json]) -> Json:
    headers = cast(dict[str, str], input_value.get("headers", {}))
    return cast(Json, logging_safe_mcp_headers(headers))


def pin(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, Json]], input_value["tools"])
    pinned = cast(dict[str, dict[str, Json]], input_value["pinned"])
    names = {cast(str, tool["name"]) for tool in tools}
    served: list[dict[str, Json]] = []
    changed: list[str] = []
    added = sorted(names - pinned.keys())
    for tool in tools:
        name = cast(str, tool["name"])
        if name not in pinned:
            continue
        entry = pinned[name]
        restored = dict(tool)
        if (
            tool.get("description") != entry.get("description")
            or tool.get("input_schema") != entry.get("input_schema")
        ):
            changed.append(name)
        restored["description"] = entry.get("description")
        restored["input_schema"] = entry.get("input_schema", {})
        served.append(restored)
    removed = sorted(set(pinned) - names)
    return cast(Json, {"served": served, "added": added, "removed": removed, "changed": changed})


def overrides(input_value: dict[str, Json]) -> Json:
    tools = cast(list[dict[str, Json]], input_value["tools"])
    configured = cast(dict[str, str], input_value["overrides"])
    prefix = cast(str, input_value.get("prefix", ""))
    result: list[dict[str, Json]] = []
    for tool in tools:
        name = cast(str, tool["name"])
        description = configured.get(name[len(prefix) + 1 :], tool.get("description"))
        result.append({**tool, "description": description})
    return cast(Json, result)

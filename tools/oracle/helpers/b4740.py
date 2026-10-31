from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def normalize(schema: Json) -> dict[str, Json]:
    value = dict(cast(dict[str, Json], schema)) if isinstance(schema, dict) else {}
    value.setdefault("type", "object")
    value.setdefault("properties", {})
    value.setdefault("additionalProperties", False)
    return value


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    tool = cast(dict[str, Json], input_value["tool"])
    schema = normalize(tool.get("inputSchema"))
    description = tool.get("description") or ""
    name = cast(str, tool["name"])
    operation = cast(str, input_value.get("operation", "mcp_tool_schema"))
    if operation == "mcp_tool_schema":
        return {
            "openai": {
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": schema,
                    "strict": False,
                },
            },
            "responses": {
                "name": name,
                "description": description,
                "parameters": schema,
                "strict": False,
                "type": "function",
            },
            "anthropic": {
                "name": name,
                "description": description,
                "input_schema": tool.get("inputSchema") or {},
                "type": "custom",
            },
        }
    raise ValueError(f"unknown operation: {operation}")

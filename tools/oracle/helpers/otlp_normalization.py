import json
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
MAX = 65536


def truncate(value: str) -> str:
    encoded = value.encode()
    if len(encoded) <= MAX:
        return value
    kept = encoded[:MAX].decode("utf-8", "ignore")
    return f"{kept}…[truncated {len(encoded) - len(kept.encode())} bytes]"


def content_text(value: Json) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if not isinstance(value, list):
        return json.dumps(value, separators=(",", ":"))
    text: list[str] = []
    for block in value:
        if not isinstance(block, dict):
            return json.dumps(value, separators=(",", ":"))
        if isinstance(block.get("text"), str):
            text.append(cast(str, block["text"]))
        elif block.get("type") not in {
            "reasoning", "thinking", "redacted_thinking", "function_call", "tool_use", "tool_call"
        }:
            return json.dumps(value, separators=(",", ":"))
    return "\n\n".join(text)


def ui_content(raw: str) -> Json:
    try:
        value = cast(Json, json.loads(raw))
    except json.JSONDecodeError:
        return {"kind": "text", "text": raw}
    values = value if isinstance(value, list) else [value] if isinstance(value, dict) else None
    if values:
        messages: list[Json] = []
        valid = True
        for raw_item in values:
            if not isinstance(raw_item, dict):
                valid = False
                break
            item = cast(dict[str, Json], raw_item)
            calls_value = item.get("tool_calls")
            calls = cast(list[Json], calls_value) if isinstance(calls_value, list) else []
            if "content" not in item and not calls:
                valid = False
                break
            role_name = str(item.get("role") or item.get("type") or "")
            role = {
                "system": "system", "user": "user", "human": "user", "assistant": "assistant",
                "ai": "assistant", "tool": "tool",
            }.get(role_name, "assistant" if calls else "user")
            message: dict[str, Json] = {"role": role, "content": content_text(item.get("content"))}
            if item.get("name") is not None:
                message["name"] = item["name"]
            if calls:
                message["tool_calls"] = calls
            messages.append(cast(Json, message))
        if valid:
            return {"kind": "messages", "messages": messages}
    if isinstance(value, dict):
        fields: list[Json] = [
            cast(Json, {"key": key, "value": field_value(item)})
            for key, item in value.items()
        ]
        return {"kind": "fields", "fields": fields}
    return {"kind": "text", "text": value if isinstance(value, str) else raw}


def field_value(item: Json) -> str:
    return item if isinstance(item, str) else json.dumps(item, separators=(",", ":"))


def run(value: dict[str, Json]) -> Json:
    operation, body = next(iter(value.items()))
    if not isinstance(body, dict):
        raise ValueError("operation body must be an object")
    if operation == "otlp_truncate":
        return truncate(str(body.get("value", "")))
    if operation == "otlp_content_text":
        return content_text(body.get("content"))
    if operation == "otlp_elision":
        return truncate(str(body.get("value", "")))
    if operation == "otlp_ui_content":
        return ui_content(str(body.get("value", "")))
    if operation == "otlp_response_encoding":
        error = body.get("error")
        if body.get("content_type") == "application/json":
            body_text = json.dumps({"message": error or ""}).encode() if error else b"{}"
            return {"body": list(body_text), "content_type": "application/json"}
        return {"body": [], "content_type": "application/x-protobuf"}
    raise ValueError("unsupported OTLP helper operation")

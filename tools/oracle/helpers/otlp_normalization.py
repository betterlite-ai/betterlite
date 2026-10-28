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


def run(value: dict[str, Json]) -> Json:
    operation, body = next(iter(value.items()))
    if not isinstance(body, dict):
        raise ValueError("operation body must be an object")
    if operation == "otlp_truncate":
        return truncate(str(body.get("value", "")))
    if operation == "otlp_content_text":
        return content_text(body.get("content"))
    if operation == "otlp_response_encoding":
        error = body.get("error")
        if body.get("content_type") == "application/json":
            body = json.dumps({"message": error or ""}) if error else "{}"
            return {"body": body, "content_type": "application/json"}
        return {"body": "", "content_type": "application/x-protobuf"}
    raise ValueError("unsupported OTLP helper operation")

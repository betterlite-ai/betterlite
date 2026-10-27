# pyright: reportAny=false, reportUnknownVariableType=false, reportUnknownMemberType=false, reportUnknownArgumentType=false, reportOptionalIterable=false, reportGeneralTypeIssues=false, reportReturnType=false
import json
from typing import Any

# The adapter intentionally mirrors LiteLLM's untyped JSON boundary.
# ruff: noqa: ANN401


def _normalize(value: Any) -> tuple[Any, list[str]]:  # noqa: C901
    if not isinstance(value, list):
        return value, []
    rewritten: set[str] = set()
    output: list[Any] = []
    for item in value:
        if not isinstance(item, dict):
            output.append(item)
            continue
        item_type = item.get("type")
        if item_type == "agent_message":
            rewritten.add(item_type)
            content = item.get("content")
            text = ""
            if isinstance(content, list):
                text = "".join(
                    str(block.get("text") or block.get("encrypted_content") or "")
                    for block in content
                    if isinstance(block, dict)
                )
            if text:
                output.append(
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": text}],
                    }
                )
        elif item_type == "context_compaction":
            rewritten.add(item_type)
            encrypted = item.get("encrypted_content")
            if isinstance(encrypted, str) and encrypted:
                output.append({"type": "compaction", "encrypted_content": encrypted})
        elif item_type == "local_shell_call":
            rewritten.add(item_type)
            call_id = item.get("call_id")
            if isinstance(call_id, str) and call_id:
                action = item.get("action") if isinstance(item.get("action"), dict) else {}
                output.append(
                    {
                        "type": "function_call",
                        "call_id": call_id,
                        "name": "local_shell",
                        "arguments": json.dumps(action),
                    }
                )
        else:
            output.append(item)
    return output, sorted(rewritten)


def _drop(tools: list[Any], supported: set[str]) -> tuple[list[Any], list[str]]:
    kept: list[Any] = []
    dropped: set[str] = set()
    for tool in tools:
        if not isinstance(tool, dict) or tool.get("type") in supported:
            kept.append(tool)
        else:
            dropped.add(str(tool.get("type")))
    return kept, sorted(dropped)


def _hoist(value: Any, tools: list[Any] | None) -> dict[str, Any]:
    existing = tuple(tools or ())
    if not isinstance(value, list):
        return {"input": value, "tools": list(existing), "hoisted": [], "debug": None}
    items = [
        item
        for item in value
        if isinstance(item, dict) and item.get("type") == "additional_tools"
    ]
    if not items:
        return {"input": value, "tools": list(existing), "hoisted": [], "debug": None}
    hoisted = [tool for item in items for tool in item.get("tools", ()) if isinstance(tool, dict)]
    remaining = [
        item
        for item in value
        if not (isinstance(item, dict) and item.get("type") == "additional_tools")
    ]
    debug = (
        "Responses API: hoisting "
        f"{len(hoisted)} tool(s) out of {len(items)} 'additional_tools' input item(s) "
        "into the top-level tools param."
    )
    return {"input": remaining, "tools": [*existing, *hoisted], "hoisted": hoisted, "debug": debug}


def run(input_value: dict[str, Any]) -> dict[str, Any]:
    operation = input_value.get("operation")
    if operation == "normalize":
        value, rewritten = _normalize(input_value.get("input"))
        return {"input": value, "rewritten_types": rewritten}
    if operation == "drop_unsupported_tools":
        tools, dropped = _drop(
            input_value.get("tools", []), set(input_value.get("supported_types", []))
        )
        return {"tools": tools, "dropped_types": dropped}
    if operation == "hoist_additional_tools":
        return _hoist(input_value.get("input"), input_value.get("tools"))
    raise ValueError("unknown codex_history_tools operation")

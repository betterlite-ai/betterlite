import json
from typing import TypeAlias, cast

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


def _object(value: JsonValue) -> JsonObject:
    return value if isinstance(value, dict) else {}


def _text(value: JsonValue) -> str:
    return value if isinstance(value, str) else ""


def _tool_events(part: JsonObject) -> list[JsonObject]:
    native = _text(part.get("tool"))
    call_id = _text(part.get("callID")) or _text(part.get("id"))
    state = _object(part.get("state"))
    raw_input = state.get("input")
    tool_input = (
        raw_input
        if isinstance(raw_input, dict)
        else {"input": raw_input}
        if raw_input is not None
        else {}
    )
    call = cast(JsonObject, {
        "type": "tool_call", "id": call_id, "name": _name(native), "native_name": native,
        "input": tool_input, "builtin": native in _BUILTINS,
    })
    if state.get("status") == "error":
        output = _text(state.get("error")) or _text(state.get("output")) or "tool failed"
        result = cast(JsonObject, {
            "type": "tool_result", "id": call_id, "output": output, "is_error": True,
        })
    else:
        output = state.get("output")
        output_text = output if isinstance(output, str) else json.dumps(
            output, separators=(",", ":"))
        result = cast(JsonObject, {
            "type": "tool_result", "id": call_id,
            "output": output_text,
            "is_error": native == "invalid",
        })
    return [call, result]


def _name(native: str) -> str:
    return {"edit": "edit", "multiedit": "edit", "patch": "edit", "apply_patch": "edit",
            "list": "ls", "webfetch": "web_search", "websearch": "web_search"}.get(native, native)


_BUILTINS = {"read", "write", "edit", "multiedit", "patch", "apply_patch", "bash", "glob", "grep",
             "list", "webfetch", "websearch", "task", "todowrite", "todoread", "skill", "invalid",
             "question", "lsp", "codesearch", "plan_enter", "plan_exit"}


def _stream(text: str) -> JsonObject:
    session_id = None
    final_text = ""
    error = None
    step_texts: list[str] = []
    events: list[JsonObject] = []
    for raw_line in text.splitlines():
        line = _object(cast(JsonValue, json.loads(raw_line)))
        if session_id is None and line.get("sessionID"):
            session_id = str(line["sessionID"])
        part = _object(line.get("part"))
        event_type = line.get("type")
        if event_type == "step_start":
            step_texts = []
        elif event_type == "text":
            value = _text(part.get("text"))
            if value:
                step_texts.append(value)
                final_text = "\n\n".join(step_texts)
                events.append(cast(JsonObject, {"type": "text", "delta": value}))
        elif event_type == "reasoning":
            value = _text(part.get("text"))
            if value:
                events.append(cast(JsonObject, {"type": "reasoning", "delta": value}))
        elif event_type == "tool_use":
            events.extend(_tool_events(part))
        elif event_type == "error":
            details = _object(line.get("error"))
            value = _text(_object(details.get("data")).get("message")) or _text(
                details.get("name")) or "opencode reported an error"
            error = f"{error}\n{value}" if error else value
    return cast(JsonObject, {"session_id": session_id, "final_text": final_text,
                             "error": error, "events": events})


def _requests(text: str) -> list[JsonObject]:
    result: list[JsonObject] = []
    for raw_line in text.splitlines():
        request = _object(cast(JsonValue, json.loads(raw_line)))
        headers = _object(request.get("headers"))
        result.append({"auth_prefix": request.get("auth_prefix"),
                       "content_type": headers.get("Content-Type"),
                       "tools": request.get("tools", []), "n_messages": request.get("n_messages"),
                       "roles": request.get("roles", [])})
    return result


def run(argument: JsonValue) -> JsonObject:
    if not isinstance(argument, dict):
        raise ValueError("b4836 input must be an object")
    stream = argument.get("stream")
    requests = argument.get("requests")
    if isinstance(stream, str) and requests is None:
        result = _stream(stream)
    elif isinstance(requests, str) and stream is None:
        result = cast(JsonObject, {"requests": _requests(requests)})
    else:
        raise ValueError("b4836 input must contain exactly one stream or requests string")
    return cast(JsonObject, {"result": result})

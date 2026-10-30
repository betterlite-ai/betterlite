import json
from typing import TypeAlias, cast

from litellm.harness.types import Reasoning, Text, ToolCall, ToolResult
from litellm.llms.opencode.harness.transformation import OpenCodeHarnessConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
JsonObject: TypeAlias = dict[str, Json]


def _object(value: Json) -> JsonObject:
    return value if isinstance(value, dict) else {}


class EventMismatchError(ValueError):
    pass


def _stream(stream: str) -> dict[str, Json]:
    config = OpenCodeHarnessConfig()
    state = config.create_stream_state()
    events: list[Json] = []
    started: set[str] = set()
    for raw_line in stream.splitlines():
        if not raw_line:
            continue
        line = cast(dict[str, Json], json.loads(raw_line))
        for event in config.transform_stream_line(line, state):
            if isinstance(event, Text):
                events.append({"kind": "text", "delta": event.delta})
            elif isinstance(event, Reasoning):
                events.append({"kind": "reasoning", "delta": event.delta})
            elif isinstance(event, ToolCall):
                started.add(event.id)
                events.append({
                    "kind": "tool_call", "id": event.id, "name": event.name,
                    "native_name": event.native_name, "builtin": event.builtin,
                    "input": cast(dict[str, Json], dict(event.input)),
                })
            elif isinstance(event, ToolResult):
                if event.id not in started:
                    raise EventMismatchError("ToolResult preceded ToolCall")
                events.append({
                    "kind": "tool_result", "id": event.id,
                    "output": event.output, "is_error": event.is_error,
                })
            else:
                raise EventMismatchError("Unexpected OpenCode event")
    return {"session_id": state.session_id, "final_text": state.final_text,
            "error": state.error, "events": events}


def _requests(text: str) -> list[dict[str, Json]]:
    result: list[dict[str, Json]] = []
    for raw_line in text.splitlines():
        request = _object(cast(Json, json.loads(raw_line)))
        headers = _object(request.get("headers"))
        result.append({"auth_prefix": request.get("auth_prefix"),
                       "content_type": headers.get("Content-Type"),
                       "tools": request.get("tools", []), "n_messages": request.get("n_messages"),
                       "roles": request.get("roles", [])})
    return result


def run(argument: Json) -> dict[str, Json]:
    if not isinstance(argument, dict):
        raise ValueError("b4836 input must be an object")
    stream = argument.get("stream")
    requests = argument.get("requests")
    if isinstance(stream, str) and requests is None:
        result: Json = _stream(stream)
    elif isinstance(requests, str) and stream is None:
        result = cast(Json, {"requests": _requests(requests)})
    else:
        raise ValueError("b4836 input must contain exactly one stream or requests string")
    return {"result": result}

import json
from typing import TypeAlias, cast

from litellm.harness.types import Reasoning, Text, ToolCall, ToolResult
from litellm.llms.codex.harness.transformation import CodexHarnessConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class EventMismatchError(ValueError):
    pass


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    stream = cast(str, input_value["stream"])
    config = CodexHarnessConfig()
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
                raise EventMismatchError("Unexpected Codex event")
    return {
        "thread_id": state.thread_id,
        "final_text": state.final_text,
        "error": state.error,
        "failed": state.failed,
        "events": events,
    }

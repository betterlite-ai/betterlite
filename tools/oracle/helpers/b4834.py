import json
from typing import TypeAlias, cast

from litellm.harness.types import Compaction, Reasoning, Text, ToolCall, ToolResult
from litellm.llms.base_llm.harness.utils import last_json_object
from litellm.llms.claude_code.harness.transformation import (
    ClaudeCodeHarnessConfig,
    ClaudeCodeStreamState,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class EventMismatchError(ValueError):
    pass


def structured_output_json(state: ClaudeCodeStreamState) -> str | None:
    if isinstance(state.structured_output, dict):
        return json.dumps(cast(dict[str, Json], state.structured_output))
    return last_json_object(state.final_text)


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    stream = cast(str, input_value["stream"])
    config = ClaudeCodeHarnessConfig()
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
            elif isinstance(event, Compaction):
                events.append({
                    "kind": "compaction", "tokens_before": event.tokens_before,
                    "tokens_after": event.tokens_after,
                })
            else:
                raise EventMismatchError("Unexpected Claude Code event")
    return {
        "session_id": state.session_id,
        "final_text": state.final_text,
        "structured_output": structured_output_json(state),
        "events": events,
    }

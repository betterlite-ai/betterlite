import json
from pathlib import Path
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> dict[str, Json]:  # noqa: C901
    fixture = Path(cast(str, input_value["fixture"]))
    thread_id = None
    final_text = ""
    error = None
    failed = False
    started: set[str] = set()
    events: list[dict[str, str]] = []
    for raw_line in fixture.read_text().splitlines():
        if not raw_line:
            continue
        event = cast(dict[str, Json], json.loads(raw_line))
        event_type = cast(str | None, event.get("type"))
        if event_type == "thread.started":
            thread_id = cast(str | None, event.get("thread_id"))
        elif event_type == "error":
            error = cast(str | None, event.get("message")) or "codex reported an error"
        elif event_type == "turn.failed":
            failed = True
            detail = event.get("error")
            detail_message = detail.get("message") if isinstance(detail, dict) else detail
            error = cast(str | None, detail_message) or error or "codex turn failed"
        elif event_type in {"item.started", "item.updated", "item.completed"}:
            item = event.get("item")
            if not isinstance(item, dict):
                continue
            kind = cast(str, item.get("type", ""))
            item_id = str(item.get("id") or "")
            if kind == "agent_message" and event_type == "item.completed":
                final_text = str(item.get("text") or "")
                events.append({"kind": "text", "id": item_id})
            elif kind == "reasoning" and event_type == "item.completed":
                events.append({"kind": "reasoning", "id": item_id})
            elif kind in {"command_execution", "file_change", "web_search", "mcp_tool_call"}:
                if item_id not in started:
                    started.add(item_id)
                    events.append({"kind": "tool_call", "id": item_id})
                if event_type == "item.completed":
                    events.append({"kind": "tool_result", "id": item_id})
    return cast(dict[str, Json], {
        "thread_id": thread_id,
        "final_text": final_text,
        "error": error,
        "failed": failed,
        "events": events,
    })

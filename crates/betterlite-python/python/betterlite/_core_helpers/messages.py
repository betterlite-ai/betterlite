from collections.abc import Iterable
from typing import TypeVar

from pydantic import JsonValue

_Item = TypeVar("_Item")


def remove_index_from_tool_calls(messages: list[dict[str, JsonValue]] | None) -> None:
    if messages is None:
        return
    for message in messages:
        tool_calls = message.get("tool_calls")
        match tool_calls:
            case list():
                raw_calls: list[JsonValue] = tool_calls
                for tool_call in raw_calls:
                    _drop_tool_call_index(tool_call)
            case object():
                pass


def _drop_tool_call_index(tool_call: JsonValue) -> None:
    match tool_call:
        case dict():
            fields: dict[str, JsonValue] = tool_call
            fields.pop("index", None)
        case str() | bool() | int() | float() | list() | None:
            pass


def remove_items_at_indices(items: list[_Item] | None, indices: Iterable[int]) -> None:
    if items is None:
        return
    for index in sorted(set(indices), reverse=True):
        if 0 <= index < len(items):
            items.pop(index)

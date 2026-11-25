from collections.abc import Iterable
from typing import TypeVar

from pydantic import JsonValue

_Item = TypeVar("_Item")


def remove_index_from_tool_calls(messages: list[dict[str, JsonValue]] | None) -> None:
    if messages is None:
        return
    for message in messages:
        match message.get("tool_calls"):
            case list() as tool_calls:
                for tool_call in tool_calls:
                    match tool_call:
                        case dict() as fields if "index" in fields:
                            fields.pop("index", None)
                        case object():
                            pass
            case object():
                pass


def remove_items_at_indices(items: list[_Item] | None, indices: Iterable[int]) -> None:
    if items is None:
        return
    for index in sorted(set(indices), reverse=True):
        if 0 <= index < len(items):
            items.pop(index)

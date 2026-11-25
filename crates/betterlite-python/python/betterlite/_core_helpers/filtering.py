from contextlib import suppress
from typing import Final

from pydantic import JsonValue

from betterlite._core_helpers.values import PythonValue, object_fields

_SKIPPED_NAMES: Final = frozenset({"Logging", "LiteLLMLoggingObj", "Router"})


# ast-grep-ignore: no-one-line-functions
def filter_exceptions_from_params(data: PythonValue, max_depth: int = 20) -> PythonValue:
    return _filtered_value(data, max_depth, frozenset())


def _filtered_value(
    data: PythonValue, max_depth: int, ancestors: frozenset[int]
) -> PythonValue:
    if max_depth <= 0:
        return data
    if _removed_in_parent(data):
        return None
    name = type(data).__name__
    if name in _SKIPPED_NAMES:
        return None
    fields = object_fields(data)
    if fields is not None:
        return _kept_fields(fields, max_depth, ancestors)
    # ast-grep-ignore: no-isinstance
    if isinstance(data, list):
        return _kept_items(data, max_depth, ancestors)
    return data


def _removed_in_parent(value: PythonValue) -> bool:
    # ast-grep-ignore: no-isinstance
    if isinstance(value, Exception):
        return True
    if type(value) is type:
        return False
    return callable(value)


def _filtered_child(
    value: PythonValue, max_depth: int, ancestors: frozenset[int]
) -> PythonValue:
    if _removed_in_parent(value):
        return None
    if id(value) in ancestors:
        return value
    with suppress(Exception):
        return _filtered_value(value, max_depth - 1, ancestors)
    return None


def _kept_fields(
    fields: dict[str, PythonValue], max_depth: int, ancestors: frozenset[int]
) -> dict[str, PythonValue]:
    kept: dict[str, PythonValue] = {}
    nested = ancestors | {id(fields)}
    for key, value in fields.items():
        filtered = _filtered_child(value, max_depth, nested)
        if filtered is not None:
            kept[key] = filtered
    return kept


def _kept_items(
    items: list[PythonValue], max_depth: int, ancestors: frozenset[int]
) -> list[PythonValue]:
    kept: list[PythonValue] = []
    for item in items:
        filtered = _filtered_child(item, max_depth, ancestors | {id(items)})
        if filtered is not None:
            kept.append(filtered)
    return kept


def filter_internal_params(
    data: dict[str, JsonValue], additional_internal_params: set[str] | None = None
) -> dict[str, JsonValue]:
    internal = {"skip_mcp_handler", "mcp_handler_context", "_skip_mcp_handler"}
    if additional_internal_params:
        internal |= additional_internal_params
    return {key: value for key, value in data.items() if key not in internal}

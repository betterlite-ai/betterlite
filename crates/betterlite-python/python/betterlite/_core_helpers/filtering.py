from contextlib import suppress
from typing import Final

from pydantic import JsonValue

from betterlite._core_helpers.values import PythonValue

_SKIPPED_NAMES: Final = frozenset({"Logging", "LiteLLMLoggingObj", "Router"})


def filter_exceptions_from_params(data: PythonValue, max_depth: int = 20) -> PythonValue:
    if max_depth <= 0:
        return data
    if _removed_in_parent(data):
        return None
    name = type(data).__name__
    if name in _SKIPPED_NAMES:
        return None
    match data:
        case dict() as fields:
            return _kept_fields(fields, max_depth)
        case list() as items:
            return _kept_items(items, max_depth)
        case object():
            return data


def _removed_in_parent(value: PythonValue) -> bool:
    match value:
        case Exception():
            return True
        case type():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L644
            return False
        case object():
            return callable(value)


def _filtered_child(value: PythonValue, max_depth: int) -> PythonValue:
    if _removed_in_parent(value):
        return None
    with suppress(Exception):
        return filter_exceptions_from_params(value, max_depth - 1)
    return None


def _kept_fields(fields: dict[str, PythonValue], max_depth: int) -> dict[str, PythonValue]:
    kept: dict[str, PythonValue] = {}
    for key, value in fields.items():
        filtered = _filtered_child(value, max_depth)
        if filtered is not None:
            kept[key] = filtered
    return kept


def _kept_items(items: list[PythonValue], max_depth: int) -> list[PythonValue]:
    kept: list[PythonValue] = []
    for item in items:
        filtered = _filtered_child(item, max_depth)
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

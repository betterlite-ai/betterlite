import copy
from contextlib import suppress
from typing import Final

import betterlite
from betterlite._core_helpers.values import PythonValue, object_fields

_PARENT_SPAN: Final = "litellm_parent_otel_span"
_PLACEHOLDER: Final = "placeholder"
_BUCKET_NAMES: Final = ("metadata", "litellm_metadata")


def safe_deep_copy(data: PythonValue) -> PythonValue:
    if betterlite.safe_memory_mode is True:
        return data
    match data:
        case dict() as fields:
            copied: dict[str, PythonValue] = {}
            for key, value in fields.items():
                sanitized = _sanitized_value(key, value)
                copied_value = _copied_or_original(sanitized)
                copied[key] = value if copied_value is sanitized else copied_value
            return copied
    return _copied_or_original(data)


def independent_snapshot(data: dict[str, PythonValue]) -> dict[str, PythonValue]:
    snapshot: dict[str, PythonValue] = {}
    for key, value in data.items():
        sanitized = _sanitized_value(key, value)
        copied = _copied_or_original(sanitized)
        if copied is sanitized:
            snapshot[key] = value
            continue
        snapshot[key] = _restored_span(copied, value) if key in _BUCKET_NAMES else copied
    return snapshot


def _sanitized_value(key: str, value: PythonValue) -> PythonValue:
    if key not in _BUCKET_NAMES:
        return value
    fields = object_fields(value)
    if fields is None:
        return value
    return {name: _PLACEHOLDER if name == _PARENT_SPAN else item for name, item in fields.items()}


def _copied_or_original(value: PythonValue) -> PythonValue:
    with suppress(Exception):
        return copy.deepcopy(value)
    return value


def _restored_span(copied: PythonValue, original: PythonValue) -> PythonValue:
    original_fields = object_fields(original)
    copied_fields = object_fields(copied)
    if original_fields is None or copied_fields is None:
        return copied
    if _PARENT_SPAN not in original_fields:
        return copied
    return {**copied_fields, _PARENT_SPAN: original_fields[_PARENT_SPAN]}

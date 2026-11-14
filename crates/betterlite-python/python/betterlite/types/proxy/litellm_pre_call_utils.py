from typing import TypedDict

from pydantic import JsonValue
from typing_extensions import override


class RedactedDict(dict[str, JsonValue]):
    @override
    def __repr__(self) -> str:
        return "RedactedDict(REDACTED)"

    @override
    def __str__(self) -> str:
        return "RedactedDict(REDACTED)"

    @override
    def copy(self) -> "RedactedDict":
        return RedactedDict(super().copy())


class SecretFields(TypedDict):
    raw_headers: dict[str, JsonValue]

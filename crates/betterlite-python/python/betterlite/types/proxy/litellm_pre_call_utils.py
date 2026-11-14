from typing import TypedDict

from pydantic import JsonValue


class RedactedDict(dict[str, JsonValue]):
    def __repr__(self) -> str:
        return "RedactedDict(REDACTED)"

    def __str__(self) -> str:
        return "RedactedDict(REDACTED)"

    def copy(self) -> "RedactedDict":
        return RedactedDict(super().copy())


class SecretFields(TypedDict):
    raw_headers: dict[str, JsonValue]

import base64
from collections.abc import Sequence
from datetime import datetime
from typing import Literal, Self, TypeAlias

JsonValue: TypeAlias = (
    bool
    | int
    | float
    | str
    | datetime
    | list["JsonValue"]
    | dict[str, "JsonValue"]
    | None
)
FieldKind: TypeAlias = Literal["plain", "base64", "aware", "model", "list", "mapping"]
ValidatorMode: TypeAlias = Literal["before", "after"]


class ValidationError(ValueError):
    fields: tuple[str, ...]

    def __init__(self, model: str, fields: Sequence[str], reason: str = "") -> None:
        self.fields = tuple(fields)
        named = ", ".join(self.fields)
        detail = f"{named}: {reason}" if named and reason else named or reason
        super().__init__(f"validation error for {model}: {detail}")


class Base64Str(str):
    __slots__ = ()

    def __new__(cls, value: str) -> Self:
        decoded = base64.b64decode(value).decode("utf-8")
        return super().__new__(cls, decoded)


def parse_datetime(value: str | datetime) -> datetime:
    match value:
        case str():
            return datetime.fromisoformat(value)
        case datetime():
            return value


class AwareDatetime(datetime):
    def __new__(cls, value: str | datetime) -> Self:
        parsed = parse_datetime(value)
        if parsed.tzinfo is None or parsed.utcoffset() is None:
            raise ValidationError("AwareDatetime", (), "timezone is missing")
        return super().__new__(
            cls,
            parsed.year,
            parsed.month,
            parsed.day,
            parsed.hour,
            parsed.minute,
            parsed.second,
            parsed.microsecond,
            tzinfo=parsed.tzinfo,
            fold=parsed.fold,
        )

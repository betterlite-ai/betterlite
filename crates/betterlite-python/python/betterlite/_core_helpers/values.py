from collections.abc import Callable, Iterator, Mapping, Sequence
from datetime import datetime
from typing import TypeAlias

from betterlite.types.utils.base import Dumpable

PythonValue: TypeAlias = (
    bool
    | int
    | float
    | str
    | BaseException
    | datetime
    | Dumpable
    | Iterator["PythonValue"]
    | Sequence["PythonValue"]
    | Mapping[str, "PythonValue"]
    | type[BaseException]
    | Callable[..., "PythonValue"]
    | None
)


def object_fields(value: PythonValue) -> dict[str, PythonValue] | None:
    match value:
        case dict() as fields:
            return fields
        case object():
            return None

from collections.abc import Callable, Iterator, Mapping, Sequence
from datetime import date, datetime
from decimal import Decimal
from typing import Final, TypeAlias

from pydantic import TypeAdapter, ValidationError
from typing_extensions import TypeIs

from betterlite.types.utils.base import Dumpable

PythonValue: TypeAlias = (  # type: ignore[explicit-any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L620
    bool
    | int
    | float
    | str
    | BaseException
    | datetime
    | date
    | Decimal
    | Dumpable
    | Iterator["PythonValue"]
    | Sequence["PythonValue"]
    | Mapping[str, "PythonValue"]
    | type[BaseException]
    | Callable[..., "PythonValue"]
    | None
)
_TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


def is_container(value: PythonValue) -> TypeIs[dict[str, PythonValue] | list[PythonValue]]:
    match value:
        case dict() | list():
            return True
        case bool() | int() | float() | str() | BaseException() | date() | Decimal():
            return False
        case Dumpable() | Iterator() | Sequence() | Mapping() | Callable() | None:
            return False


def object_fields(value: PythonValue) -> dict[str, PythonValue] | None:
    if not is_container(value):
        return None
    match value:
        case dict():
            return value
        case list():
            return None


def text_of(value: PythonValue) -> str | None:
    try:
        return _TEXT.validate_python(value, strict=True)
    except ValidationError:
        return None

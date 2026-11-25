from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from datetime import datetime
from typing import Final, Protocol, TypeAlias, runtime_checkable

from pydantic import TypeAdapter, ValidationError

from betterlite.types.utils.base import Dumpable

PythonValue: TypeAlias = (  # type: ignore[explicit-any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/core_helpers.py#L620
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
_TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


@runtime_checkable
class CallableLeaf(Protocol):
    def __call__(self, *args: PythonValue) -> PythonValue: ...


def object_fields(value: PythonValue) -> dict[str, PythonValue] | None:
    match value:
        case dict() as fields:
            return fields
        case (
            int() | float() | BaseException() | datetime()
            | Dumpable() | Iterable() | CallableLeaf() | None
        ):
            return None


def text_of(value: PythonValue) -> str | None:
    try:
        return _TEXT.validate_python(value, strict=True)
    except ValidationError:
        return None

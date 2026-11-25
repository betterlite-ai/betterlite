from collections.abc import Callable, Iterator, Mapping, Sequence
from datetime import datetime
from typing import TypeAlias

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


def object_fields(value: PythonValue) -> dict[str, PythonValue] | None:
    match value:
        case dict() as fields:
            return fields
    return None

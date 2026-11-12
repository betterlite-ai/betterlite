from collections.abc import Mapping, Sequence
from typing import Protocol, TypeAlias, runtime_checkable

from betterlite.types.utils.json_types import JsonValue


@runtime_checkable
class ModelObject(Protocol):
    def model_dump(
        self,
        *,
        exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue: ...


StoredValue: TypeAlias = (
    JsonValue
    | ModelObject
    | Sequence["StoredValue"]
    | Mapping[str, "StoredValue"]
)

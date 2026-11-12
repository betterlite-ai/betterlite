from typing import Generic, Literal, TypeVar

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject, dump_any, json_ready
from betterlite.types.utils.json_types import JsonValue
from betterlite.types.utils.values import StoredValue

RootValue = TypeVar("RootValue", bound=StoredValue)


class RootModel(ResponseObject, Generic[RootValue]):
    root: RootValue

    @override
    def __init__(self, root: RootValue) -> None:
        super().__init__(root=root)

    @override
    def model_dump(
        self,
        *,
        mode: Literal["python", "json"] = "python",
        exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue:
        dumped = dump_any(self.root, exclude_unset=exclude_unset, exclude_none=exclude_none)
        if mode == "json":
            return json_ready(dumped)
        return dumped

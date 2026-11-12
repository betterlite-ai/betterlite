from collections.abc import Callable
from typing import ClassVar, ParamSpec, Protocol, cast

from betterlite.types.utils.json_types import ValidatorMode
from betterlite.types.utils.values import StoredValue

Parameters = ParamSpec("Parameters")


class StoredFunction(Protocol):
    def __call__(self, *args: StoredValue, **kwargs: StoredValue) -> StoredValue: ...


class ValidatorHost(Protocol):
    __field_validators__: ClassVar[tuple["FieldValidator", ...]]
    __model_validators__: ClassVar[tuple["ModelValidator", ...]]


class FieldValidator:
    names: tuple[str, ...]
    mode: ValidatorMode
    function: StoredFunction
    attribute: str

    def __init__(
        self,
        names: tuple[str, ...],
        mode: ValidatorMode,
        function: StoredFunction,
    ) -> None:
        self.names = names
        self.mode = mode
        self.function = function
        self.attribute = ""

    def __set_name__(self, owner: "type[ValidatorHost]", name: str) -> None:
        self.attribute = name
        owner.__field_validators__ = (*owner.__field_validators__, self)

class ModelValidator:
    mode: ValidatorMode
    function: StoredFunction
    attribute: str

    def __init__(self, mode: ValidatorMode, function: StoredFunction) -> None:
        self.mode = mode
        self.function = function
        self.attribute = ""

    def __set_name__(self, owner: "type[ValidatorHost]", name: str) -> None:
        self.attribute = name
        owner.__model_validators__ = (*owner.__model_validators__, self)

class FieldValidatorDecorator:
    names: tuple[str, ...]
    mode: ValidatorMode

    def __init__(self, *names: str, mode: ValidatorMode = "after") -> None:
        self.names = names
        self.mode = mode

    def __call__(self, function: Callable[Parameters, StoredValue]) -> FieldValidator:
        stored = cast("StoredFunction", function)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1135-L1146
        return FieldValidator(self.names, self.mode, stored)


class ModelValidatorDecorator:
    mode: ValidatorMode

    def __init__(self, *, mode: ValidatorMode = "after") -> None:
        self.mode = mode

    def __call__(self, function: Callable[Parameters, StoredValue]) -> ModelValidator:
        stored = cast("StoredFunction", function)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1487-L1489
        return ModelValidator(self.mode, stored)


field_validator = FieldValidatorDecorator
model_validator = ModelValidatorDecorator

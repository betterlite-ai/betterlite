import base64
import types
from collections.abc import Callable, Mapping, Sequence
from datetime import datetime
from types import NoneType
from typing import (
    ClassVar,
    Literal,
    ParamSpec,
    Protocol,
    Self,
    TypeAlias,
    Union,
    cast,
    get_args,
    get_origin,
    runtime_checkable,
)

JsonValue: TypeAlias = (
    bool | int | float | str | datetime | list["JsonValue"] | dict[str, "JsonValue"] | None
)


@runtime_checkable
class ModelObject(Protocol):
    def model_dump(
        self, *, exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue: ...


StoredValue: TypeAlias = (
    JsonValue | ModelObject | Sequence["StoredValue"] | Mapping[str, "StoredValue"]
)
ValidatorMode: TypeAlias = Literal["before", "after"]
AnnotationValue: TypeAlias = type | str | types.GenericAlias | types.UnionType


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
        return super().__new__(cls, base64.b64decode(value).decode("utf-8"))


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
            cls, parsed.year, parsed.month, parsed.day, parsed.hour, parsed.minute,
            parsed.second, parsed.microsecond, tzinfo=parsed.tzinfo, fold=parsed.fold,
        )


Parameters = ParamSpec("Parameters")


class StoredFunction(Protocol):
    def __call__(self, *args: StoredValue, **kwargs: StoredValue) -> StoredValue: ...


class ValidatorHost(Protocol):
    __field_validators__: ClassVar[tuple["Validator", ...]]
    __model_validators__: ClassVar[tuple["Validator", ...]]


class Validator:
    names: tuple[str, ...]
    mode: ValidatorMode
    function: StoredFunction
    attribute: str
    model_level: bool

    def __init__(
        self, names: tuple[str, ...], mode: ValidatorMode, function: StoredFunction, *,
        model_level: bool,
    ) -> None:
        self.names = names
        self.mode = mode
        self.function = function
        self.attribute = ""
        self.model_level = model_level

    def __set_name__(self, owner: "type[ValidatorHost]", name: str) -> None:
        self.attribute = name
        if self.model_level:
            owner.__model_validators__ = (*owner.__model_validators__, self)
        else:
            owner.__field_validators__ = (*owner.__field_validators__, self)


class FieldValidatorDecorator:
    names: tuple[str, ...]
    mode: ValidatorMode

    def __init__(self, *names: str, mode: ValidatorMode = "after") -> None:
        self.names = names
        self.mode = mode

    def __call__(self, function: Callable[Parameters, StoredValue]) -> Validator:
        stored = cast("StoredFunction", function)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1135-L1146
        return Validator(self.names, self.mode, stored, model_level=False)


class ModelValidatorDecorator:
    mode: ValidatorMode

    def __init__(self, *, mode: ValidatorMode = "after") -> None:
        self.mode = mode

    def __call__(self, function: Callable[Parameters, StoredValue]) -> Validator:
        stored = cast("StoredFunction", function)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1487-L1489
        return Validator((), self.mode, stored, model_level=True)


field_validator = FieldValidatorDecorator
model_validator = ModelValidatorDecorator


class ModelBuilder(Protocol):
    def __call__(self, **data: StoredValue) -> ModelObject: ...


def allows_none(annotation: AnnotationValue) -> bool:
    if annotation is JsonValue or annotation is NoneType:
        return True
    origin = cast("type | None", get_origin(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    if origin not in (Union, types.UnionType):
        return False
    parameters = cast("tuple[AnnotationValue, ...]", get_args(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    return any(allows_none(member) for member in parameters)


def checked_value(
    annotation: AnnotationValue, value: StoredValue, model: str, name: str,
) -> StoredValue:
    if value is None or annotation is JsonValue:
        if value is None and not allows_none(annotation):
            raise ValidationError(model, (name,), "value is required")
        return value
    origin = cast("type | None", get_origin(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    if origin in (Union, types.UnionType):
        return union_value(annotation, value, model, name)
    return member_value(annotation, origin, value, model, name)


def union_value(
    annotation: AnnotationValue, value: StoredValue, model: str, name: str,
) -> StoredValue:
    parameters = cast("tuple[AnnotationValue, ...]", get_args(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    members = [item for item in parameters if item is not NoneType]
    if len(members) != 1:
        return value
    return checked_value(members[0], value, model, name)


def member_value(
    annotation: AnnotationValue, origin: type | None, value: StoredValue, model: str, name: str,
) -> StoredValue:
    if annotation is Base64Str or annotation is AwareDatetime:
        return text_value(annotation, value, model, name)
    if "__fields__" in dir(annotation):
        return model_value(annotation, value, model, name)
    if origin in (list, tuple, Sequence, dict, Mapping):
        return container_value(annotation, origin, value, model, name)
    return value


def text_value(
    annotation: AnnotationValue, value: StoredValue, model: str, name: str,
) -> StoredValue:
    match value:
        case str() if annotation is Base64Str:
            try:
                return Base64Str(value)
            except ValueError as error:
                raise ValidationError(model, (name,), str(error)) from error
        case str() | datetime() if annotation is AwareDatetime:
            try:
                return AwareDatetime(value)
            except ValidationError as error:
                raise ValidationError(model, (name,), str(error)) from error
        case _other:
            raise ValidationError(model, (name,), "wrong type")


def model_value(
    annotation: AnnotationValue, value: StoredValue, model: str, name: str,
) -> StoredValue:
    constructor = cast("type[ModelObject]", annotation)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1390
    match value:
        case Mapping():
            builder = cast("ModelBuilder", constructor)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1679
            return builder(**value)
        case _other:
            if constructor in value.__class__.__mro__:
                return value
            raise ValidationError(model, (name,), "wrong type")


def container_value(
    annotation: AnnotationValue, origin: type | None, value: StoredValue, model: str, name: str,
) -> StoredValue:
    parameters = cast("tuple[AnnotationValue, ...]", get_args(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    match value:
        case Mapping() if origin in (dict, Mapping):
            return {
                key: checked_value(parameters[1], entry, model, name)
                for key, entry in value.items()
            }
        case Sequence() if origin in (list, tuple, Sequence):
            return [checked_value(parameters[0], entry, model, name) for entry in value]
        case _other:
            raise ValidationError(model, (name,), "wrong type")


def as_data_mapping(value: StoredValue, model: str, attribute: str) -> dict[str, StoredValue]:
    match value:
        case Mapping():
            return {str(name): item for name, item in value.items()}
        case _other:
            raise ValidationError(model, (), f"{attribute} did not return a mapping")

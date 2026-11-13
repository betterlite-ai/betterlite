from collections.abc import Callable, Mapping, Sequence
from typing import (
    ClassVar,
    Literal,
    Protocol,
    TypeAlias,
    TypedDict,
    TypeVar,
    Unpack,
    overload,
    runtime_checkable,
)

from pydantic import BaseModel, ConfigDict, JsonValue
from pydantic.main import IncEx
from typing_extensions import override

ModelType = TypeVar("ModelType", bound="ResponseObject")
JsonCallValue: TypeAlias = JsonValue | IncEx | Callable[[JsonValue], JsonValue]


class DumpOptions(TypedDict, total=False):
    mode: str
    include: IncEx | None
    exclude: IncEx | None
    context: JsonValue
    by_alias: bool | None
    exclude_unset: bool
    exclude_defaults: bool
    exclude_none: bool
    exclude_computed_fields: bool
    round_trip: bool
    warnings: bool | Literal["none", "warn", "error"]
    fallback: Callable[[JsonValue], JsonValue] | None
    serialize_as_any: bool
    polymorphic_serialization: bool | None


class JsonDumpOptions(DumpOptions, total=False):
    indent: int | None
    ensure_ascii: bool


@runtime_checkable
class Dumpable(Protocol):
    def model_dump(
        self, *, mode: str = ..., include: IncEx | None = ..., exclude: IncEx | None = ...,
        exclude_unset: bool = ..., exclude_none: bool = ...,
    ) -> dict[str, JsonValue]: ...


ModelValue: TypeAlias = JsonValue | Dumpable
InitValue: TypeAlias = (
    JsonValue | Dumpable | Sequence["InitValue"] | Mapping[str, "InitValue"]
)


def dump_value(value: InitValue, *, exclude_unset: bool = False) -> InitValue:
    if value is None:
        return None
    match value:
        case Dumpable():
            dumped: dict[str, JsonValue] = value.model_dump(exclude_unset=exclude_unset)
            return dumped
        case Mapping() | Sequence() | bool() | int() | float():
            return value


def validated_value(value: InitValue, model: type[ModelType]) -> ModelType:
    if value is None:
        raise TypeError("a value must be a mapping or a model")
    match value:
        case Mapping():
            fields: dict[str, InitValue] = dict(value)
            return model.model_validate(fields)
        case Dumpable():
            dumped: dict[str, JsonValue] = value.model_dump()
            return model.model_validate(dumped)
        case bool() | int() | float() | Sequence():
            raise TypeError("a value must be a mapping or a model")


class ResponseObject(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    def __getitem__(self, key: str) -> InitValue:
        extra: dict[str, JsonValue] | None = self.model_extra
        if extra is not None and key in extra:
            found: InitValue = extra[key]
            return found
        attribute: InitValue = self.__getattribute__(key)
        return attribute

    def __setitem__(self, key: str, value: InitValue) -> None:
        self.__setattr__(key, value)

    def __delitem__(self, key: str) -> None:
        self.__delattr__(key)

    @override
    def __delattr__(self, name: str) -> None:
        try:
            super().__delattr__(name)
        except AttributeError:
            return

    def get(self, key: str, default: InitValue = None) -> InitValue:
        try:
            return self[key]
        except AttributeError:
            return default

    def __contains__(self, key: str) -> bool:
        try:
            _ = self[key]
        except AttributeError:
            return False
        return True

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = super().model_dump(**kwargs)
        return dumped

    @override
    def dict(
        self, *, include: IncEx | None = None, exclude: IncEx | None = None,
        by_alias: bool = False, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
    ) -> dict[str, JsonValue]:
        return self.model_dump(
            include=include, exclude=exclude, by_alias=by_alias,
            exclude_unset=exclude_unset, exclude_defaults=exclude_defaults,
            exclude_none=exclude_none,
        )

    @override
    def json(
        self, *, include: IncEx | None = None, exclude: IncEx | None = None,
        by_alias: bool = False, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: JsonValue,
    ) -> str:
        # pydantic's json() text without its deprecation warning; the dump classes below override it
        if encoder is not None or models_as_dict or dumps_kwargs:
            raise TypeError("json() takes no encoder, models_as_dict or extra arguments")
        return self.model_dump_json(
            include=include, exclude=exclude, by_alias=by_alias,
            exclude_unset=exclude_unset, exclude_defaults=exclude_defaults,
            exclude_none=exclude_none,
        )


class DictJsonObject(ResponseObject):
    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    @overload
    def json(self, *args: JsonValue) -> dict[str, JsonValue]: ...

    @overload
    def json(
        self, *, include: IncEx | None = None, exclude: IncEx | None = None,
        by_alias: bool = False, exclude_unset: bool = False,
        exclude_defaults: bool = False, exclude_none: bool = False,
        encoder: Callable[[JsonValue], JsonValue] | None = None,
        models_as_dict: bool = False, **dumps_kwargs: JsonValue,
    ) -> str: ...

    @override
    def json(self, *_args: JsonValue, **_kwargs: JsonCallValue) -> JsonValue:
        # LiteLLM dumps here; the second overload matches pydantic's deprecated signature
        if _args:
            raise TypeError("json() takes no positional arguments")
        dumped: dict[str, JsonValue] = self.model_dump()
        return dumped

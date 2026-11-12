import base64
import json
from collections.abc import Collection, Mapping, Sequence
from copy import deepcopy
from datetime import datetime
from typing import ClassVar, Generic, Literal, Protocol, TypeVar

from typing_extensions import override

from betterlite.types.utils.values import (
    MODEL_TYPES,
    AnnotationValue,
    Base64Str,
    JsonValue,
    StoredValue,
    ValidationError,
    Validator,
    allows_none,
    as_data_mapping,
    checked_value,
)

INTERNAL_NAMES = ("_values", "_fields_set")


class Dumpable(Protocol):
    def dict(
        self, *, include: Collection[str] | None = None,
        exclude: Collection[str] | None = None, exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue: ...

    def json(self, *, exclude_unset: bool = False, exclude_none: bool = False) -> JsonValue: ...


class ResponseObject(Dumpable):
    __fields__: ClassVar[dict[str, AnnotationValue]] = {}
    __defaults__: ClassVar[dict[str, StoredValue]] = {}
    __field_validators__: ClassVar[tuple[Validator, ...]] = ()
    __model_validators__: ClassVar[tuple[Validator, ...]] = ()
    defaults: ClassVar[dict[str, StoredValue]] = {}
    _values: dict[str, StoredValue]
    _fields_set: set[str]

    def __getattr__(self, name: str) -> StoredValue:
        if name in INTERNAL_NAMES or name not in self._values:
            raise AttributeError(name)
        return self._values[name]

    @override
    def __setattr__(self, name: str, value: StoredValue) -> None:
        if name in INTERNAL_NAMES:
            object.__setattr__(self, name, value)
            return
        self._values[name] = value
        if not name.startswith("_"):
            self._fields_set.add(name)

    @override
    def __delattr__(self, name: str) -> None:
        values = self._values
        if name in values:
            del values[name]
            return
        try:
            super().__delattr__(name)
        except AttributeError:
            return

    def __getitem__(self, key: str) -> StoredValue:
        return self.__getattr__(key)

    def __setitem__(self, key: str, value: StoredValue) -> None:
        self.__setattr__(key, value)

    def __delitem__(self, key: str) -> None:
        self.__delattr__(key)

    def get(self, key: str, default: StoredValue = None) -> StoredValue:
        values = self._values
        if key in values:
            return values[key]
        return default

    def __contains__(self, key: str) -> bool:
        return key in self._values

    @override
    def __eq__(self, other: object) -> bool:  # ast-grep-ignore: no-loose-types
        match other:
            case ResponseObject():
                return self.__class__ is other.__class__ and self._values == other._values
            case _other:
                return False

    @override
    def __hash__(self) -> int:
        raise TypeError("response objects are not hashable")

    @override
    def __init_subclass__(cls, **kwargs: StoredValue) -> None:
        super().__init_subclass__(**kwargs)
        fields = dict(cls.__fields__)
        defaults = dict(cls.__defaults__)
        annotations: dict[str, AnnotationValue] = cls.__annotations__
        for name, annotation in annotations.items():
            if name == "defaults":
                continue
            fields[name] = annotation
            if name not in defaults and allows_none(annotation):
                defaults[name] = None
        defaults.update(cls.defaults)
        cls.__fields__ = fields
        cls.__defaults__ = defaults
        MODEL_TYPES[cls] = cls

    def __init__(self, **data: StoredValue) -> None:
        self._values = {}
        self._fields_set = set[str]()
        prepared = self._before_data(data)
        fields = type(self).__fields__
        defaults = type(self).__defaults__
        missing = [name for name in fields if name not in prepared and name not in defaults]
        if missing:
            raise ValidationError(type(self).__name__, missing)
        for name in fields:
            if name in prepared:
                self._values[name] = self._checked_field(name, prepared[name])
                if not name.startswith("_"):
                    self._fields_set.add(name)
            else:
                self._values[name] = deepcopy(defaults[name])
        for name, value in prepared.items():
            if name not in fields:
                self._values[name] = value
                if not name.startswith("_"):
                    self._fields_set.add(name)
        for validator in type(self).__model_validators__:
            if validator.mode == "after":
                self._after_validator(validator)

    def _before_data(self, data: dict[str, StoredValue]) -> dict[str, StoredValue]:
        prepared = dict(data)
        for validator in type(self).__model_validators__:
            if validator.mode == "before":
                try:
                    result = validator.function(self, prepared)
                except ValueError as error:
                    raise ValidationError(type(self).__name__, (), str(error)) from error
                prepared = as_data_mapping(result, type(self).__name__, validator.attribute)
        return prepared

    def _after_validator(self, validator: Validator) -> None:
        try:
            validator.function(self)
        except ValueError as error:
            raise ValidationError(type(self).__name__, (), str(error)) from error

    def _checked_field(self, name: str, value: StoredValue) -> StoredValue:
        converted = value
        for validator in type(self).__field_validators__:
            if name in validator.names and validator.mode == "before":
                converted = self._field_validator(validator, converted)
        annotation = type(self).__fields__[name]
        converted = checked_value(annotation, converted, type(self).__name__, name)
        for validator in type(self).__field_validators__:
            if name in validator.names and validator.mode == "after":
                converted = self._field_validator(validator, converted)
        return converted

    def _field_validator(self, validator: Validator, value: StoredValue) -> StoredValue:
        try:
            return validator.function(self, value)
        except ValueError as error:
            raise ValidationError(type(self).__name__, validator.names, str(error)) from error

    def model_dump(
        self, *, mode: Literal["python", "json"] = "python",
        include: Collection[str] | None = None, exclude: Collection[str] | None = None,
        exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue:
        dumped: dict[str, JsonValue] = {}
        for name, value in self._values.items():
            if name.startswith("_"):
                continue
            if include is not None and name not in include:
                continue
            if exclude is not None and name in exclude:
                continue
            if exclude_unset and name not in self._fields_set:
                continue
            if exclude_none and value is None:
                continue
            dumped[name] = dump_any(value, exclude_unset=exclude_unset, exclude_none=exclude_none)
        return json_ready(dumped) if mode == "json" else dumped

    def model_dump_json(
        self, *, include: Collection[str] | None = None,
        exclude: Collection[str] | None = None, exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> str:
        dumped = self.model_dump(
            include=include, exclude=exclude, exclude_unset=exclude_unset,
            exclude_none=exclude_none,
        )
        return json.dumps(json_ready(dumped))

    @override
    def dict(
        self, *, include: Collection[str] | None = None,
        exclude: Collection[str] | None = None, exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue:
        return self.model_dump(
            include=include, exclude=exclude, exclude_unset=exclude_unset,
            exclude_none=exclude_none,
        )

    @override
    def json(self, *, exclude_unset: bool = False, exclude_none: bool = False) -> JsonValue:
        return self.model_dump_json(exclude_unset=exclude_unset, exclude_none=exclude_none)


def dump_any(value: StoredValue, *, exclude_unset: bool, exclude_none: bool) -> JsonValue:
    match value:
        case ResponseObject():
            return value.model_dump(exclude_unset=exclude_unset, exclude_none=exclude_none)
        case Base64Str():
            return base64.b64encode(value.encode("utf-8")).decode("ascii")
        case bool() | int() | float() | str() | datetime() | None:
            return value
        case Sequence():
            return [
                dump_any(item, exclude_unset=exclude_unset, exclude_none=exclude_none)
                for item in value
            ]
        case Mapping():
            return {
                str(key): dump_any(item, exclude_unset=exclude_unset, exclude_none=exclude_none)
                for key, item in value.items()
            }
        case _other:
            return value.model_dump(exclude_unset=exclude_unset, exclude_none=exclude_none)


def json_ready(value: JsonValue) -> JsonValue:
    match value:
        case datetime():
            return value.isoformat()
        case bool() | int() | float() | str() | None:
            return value
        case list():
            return [json_ready(item) for item in value]
        case dict():
            return {key: json_ready(item) for key, item in value.items()}


RootValue = TypeVar("RootValue", bound=StoredValue)


class RootModel(ResponseObject, Generic[RootValue]):
    root: RootValue

    @override
    def __init__(self, root: RootValue) -> None:
        super().__init__(root=root)

    @override
    def model_dump(
        self, *, mode: Literal["python", "json"] = "python",
        include: Collection[str] | None = None, exclude: Collection[str] | None = None,
        exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue:
        dumped = dump_any(self.root, exclude_unset=exclude_unset, exclude_none=exclude_none)
        return json_ready(dumped) if mode == "json" else dumped

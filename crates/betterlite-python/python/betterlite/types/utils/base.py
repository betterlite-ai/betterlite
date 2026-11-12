import base64
import json
from collections.abc import Mapping, Sequence
from copy import deepcopy
from datetime import datetime
from typing import ClassVar, Literal, cast

from typing_extensions import override

from betterlite.types.utils.accessors import AttributeAccess
from betterlite.types.utils.conversion import (
    AnnotationValue,
    FieldRule,
    as_data_mapping,
    convert_value,
    field_rule,
)
from betterlite.types.utils.json_types import Base64Str, JsonValue, ValidationError
from betterlite.types.utils.validation import FieldValidator, ModelValidator
from betterlite.types.utils.values import ModelObject, StoredValue


class ResponseObject(AttributeAccess):
    __fields__: ClassVar[dict[str, AnnotationValue]] = {}
    __defaults__: ClassVar[dict[str, StoredValue]] = {}
    __rules__: ClassVar[dict[str, FieldRule]] = {}
    __field_validators__: ClassVar[tuple[FieldValidator, ...]] = ()
    __model_validators__: ClassVar[tuple[ModelValidator, ...]] = ()
    defaults: ClassVar[dict[str, StoredValue]] = {}

    @override
    def __init_subclass__(cls, **kwargs: StoredValue) -> None:
        super().__init_subclass__(**kwargs)
        fields = dict(cls.__fields__)
        defaults = dict(cls.__defaults__)
        rules = dict(cls.__rules__)
        annotations = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1390
            "dict[str, AnnotationValue]",
            cls.__annotations__,
        )
        for name, annotation in annotations.items():
            if name == "defaults":
                continue
            fields[name] = annotation
            rules[name] = field_rule(annotation)
        defaults.update(cls.defaults)
        cls.__fields__ = fields
        cls.__defaults__ = defaults
        cls.__rules__ = rules

    def __init__(self, **data: StoredValue) -> None:
        empty: dict[str, StoredValue] = {}
        self._values = empty
        self._fields_set = set[str]()
        prepared = self._before_data(data)
        fields = type(self).__fields__
        defaults = type(self).__defaults__
        missing = [name for name in fields if name not in prepared and name not in defaults]
        if missing:
            raise ValidationError(type(self).__name__, missing)
        values = self._values
        fields_set = self._fields_set
        for name in fields:
            if name in prepared:
                values[name] = self._checked_field(name, prepared[name])
                if not name.startswith("_"):
                    fields_set.add(name)
            else:
                values[name] = deepcopy(defaults[name])
        for name, value in prepared.items():
            if name in fields:
                continue
            values[name] = value
            if not name.startswith("_"):
                fields_set.add(name)
        for validator in type(self).__model_validators__:
            if validator.mode == "after":
                self._after_validator(validator)

    def _before_data(self, data: dict[str, StoredValue]) -> dict[str, StoredValue]:
        prepared = dict(data)
        for validator in type(self).__model_validators__:
            if validator.mode != "before":
                continue
            try:
                result = validator.function(self, prepared)
            except ValueError as error:
                raise ValidationError(type(self).__name__, (), str(error)) from error
            prepared = as_data_mapping(result, type(self).__name__, validator.attribute)
        return prepared

    def _after_validator(self, validator: ModelValidator) -> None:
        try:
            validator.function(self)
        except ValueError as error:
            raise ValidationError(type(self).__name__, (), str(error)) from error

    def _checked_field(self, name: str, value: StoredValue) -> StoredValue:
        converted = value
        for validator in type(self).__field_validators__:
            if name in validator.names and validator.mode == "before":
                converted = self._field_validator(validator, converted)
        converted = convert_value(type(self).__rules__[name], converted, type(self).__name__, name)
        for validator in type(self).__field_validators__:
            if name in validator.names and validator.mode == "after":
                converted = self._field_validator(validator, converted)
        return converted

    def _field_validator(self, validator: FieldValidator, value: StoredValue) -> StoredValue:
        try:
            return validator.function(self, value)
        except ValueError as error:
            raise ValidationError(type(self).__name__, validator.names, str(error)) from error

    def model_dump(
        self,
        *,
        mode: Literal["python", "json"] = "python",
        exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> JsonValue:
        dumped = self._dumped(exclude_unset=exclude_unset, exclude_none=exclude_none)
        if mode == "json":
            return json_ready(dumped)
        return dumped

    def _dumped(self, *, exclude_unset: bool, exclude_none: bool) -> dict[str, JsonValue]:
        fields_set = self._fields_set
        dumped: dict[str, JsonValue] = {}
        for name, value in self._values.items():
            if name.startswith("_"):
                continue
            if exclude_unset and name not in fields_set:
                continue
            if exclude_none and value is None:
                continue
            dumped[name] = dump_any(value, exclude_unset=exclude_unset, exclude_none=exclude_none)
        return dumped

    def model_dump_json(
        self,
        *,
        exclude_unset: bool = False,
        exclude_none: bool = False,
    ) -> str:
        dumped = self.model_dump(exclude_unset=exclude_unset, exclude_none=exclude_none)
        return json.dumps(json_ready(dumped))

    def dict(self, **_kwargs: JsonValue) -> JsonValue:
        return self.model_dump()

    def json(self, **_kwargs: JsonValue) -> JsonValue:
        return self.model_dump_json()


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
        case ModelObject():
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

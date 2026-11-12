import types
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from types import NoneType
from typing import Protocol, TypeAlias, Union, cast, get_args, get_origin

from betterlite.types.utils.json_types import (
    AwareDatetime,
    Base64Str,
    FieldKind,
    ValidationError,
)
from betterlite.types.utils.values import ModelObject, StoredValue

AnnotationValue: TypeAlias = type | str | types.GenericAlias | types.UnionType


class ModelBuilder(Protocol):
    def __call__(self, **data: StoredValue) -> ModelObject: ...


@dataclass(frozen=True)
class FieldRule:
    kind: FieldKind = "plain"
    item: "FieldRule | None" = None
    optional: bool = False
    model: type[ModelObject] | None = None


def field_rule(annotation: AnnotationValue) -> FieldRule:
    if annotation is Base64Str:
        return FieldRule(kind="base64")
    if annotation is AwareDatetime:
        return FieldRule(kind="aware")
    if "__fields__" in dir(annotation):
        model = cast("type[ModelObject]", annotation)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1390
        return FieldRule(kind="model", model=model)
    return container_rule(annotation)


def container_rule(annotation: AnnotationValue) -> FieldRule:
    origin = cast("type | None", get_origin(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    parameters = cast("tuple[AnnotationValue, ...]", get_args(annotation))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3648
    if origin in (Union, types.UnionType):
        members = [item for item in parameters if item is not NoneType]
        optional = len(members) != len(parameters)
        if len(members) == 1:
            inner = field_rule(members[0])
            return FieldRule(
                kind=inner.kind,
                item=inner.item,
                optional=optional,
                model=inner.model,
            )
        return FieldRule(optional=optional)
    if origin is list or origin is tuple or origin is Sequence:
        return FieldRule(kind="list", item=field_rule(parameters[0]))
    if origin is dict or origin is Mapping:
        return FieldRule(kind="mapping", item=field_rule(parameters[1]))
    return FieldRule()


def convert_value(rule: FieldRule, value: StoredValue, model: str, name: str) -> StoredValue:
    if rule.kind == "plain":
        return value
    if value is None:
        return none_value(rule, model, name)
    if rule.kind == "base64":
        return base64_value(value, model, name)
    if rule.kind == "aware":
        return aware_value(value, model, name)
    if rule.kind == "model":
        return model_value(rule, value, model, name)
    return collection_value(rule, value, model, name)


def none_value(rule: FieldRule, model: str, name: str) -> StoredValue:
    if rule.optional:
        return None
    raise ValidationError(model, (name,), "value is required")


def base64_text(value: str, model: str, name: str) -> StoredValue:
    try:
        return Base64Str(value)
    except ValueError as error:
        raise ValidationError(model, (name,), str(error)) from error


def base64_value(value: StoredValue, model: str, name: str) -> StoredValue:
    match value:
        case str():
            return base64_text(value, model, name)
        case _other:
            raise ValidationError(model, (name,), "wrong type")


def aware_text(value: str | datetime, model: str, name: str) -> StoredValue:
    try:
        return AwareDatetime(value)
    except ValidationError as error:
        raise ValidationError(model, (name,), str(error)) from error


def aware_value(value: StoredValue, model: str, name: str) -> StoredValue:
    match value:
        case str() | datetime():
            return aware_text(value, model, name)
        case _other:
            raise ValidationError(model, (name,), "wrong type")


def model_value(rule: FieldRule, value: StoredValue, model: str, name: str) -> StoredValue:
    constructor = rule.model
    if constructor is None:
        raise ValidationError(model, (name,), "wrong type")
    match value:
        case Mapping():
            builder = cast("ModelBuilder", constructor)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1679
            return builder(**value)
        case _other:
            if constructor in value.__class__.__mro__:
                return value
            raise ValidationError(model, (name,), "wrong type")


def collection_value(rule: FieldRule, value: StoredValue, model: str, name: str) -> StoredValue:
    match value:
        case Mapping() if rule.kind == "mapping" and rule.item is not None:
            return {
                entry_name: convert_value(rule.item, entry, model, name)
                for entry_name, entry in value.items()
            }
        case Sequence() if rule.kind == "list" and rule.item is not None:
            return [convert_value(rule.item, entry, model, name) for entry in value]
        case _other:
            raise ValidationError(model, (name,), "wrong type")


def as_data_mapping(value: StoredValue, model: str, attribute: str) -> dict[str, StoredValue]:
    match value:
        case Mapping():
            return {str(name): item for name, item in value.items()}
        case _other:
            raise ValidationError(model, (), f"{attribute} did not return a mapping")

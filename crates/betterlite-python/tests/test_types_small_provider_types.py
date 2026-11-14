import importlib
import inspect
from enum import Enum
from types import ModuleType
from typing import Final, TypeVar, get_args

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypeForm, is_typeddict

from betterlite.types.llms import base as betterlite_base
from betterlite.types.llms.base import HiddenParams, LiteLLMPydanticObjectBase
from betterlite.types.llms.custom_http import VerifyTypes
from betterlite.types.utils.base import InitValue

MODULES: Final = (
    "aiml", "azure", "azure_ai", "base", "cohere", "custom_http", "databricks",
    "langgraph", "meta", "mistral", "ollama", "openrouter", "recraft", "rerank",
    "stability", "triton", "xai",
)
CONSTANTS: Final = {
    "azure": (
        "API_VERSION_YEAR_SUPPORTED_RESPONSE_FORMAT",
        "API_VERSION_MONTH_SUPPORTED_RESPONSE_FORMAT",
    ),
    "stability": (
        "OPENAI_SIZE_TO_STABILITY_ASPECT_RATIO",
        "STABILITY_ASPECT_RATIOS",
        "STABILITY_GENERATION_MODELS",
        "STABILITY_EDIT_ENDPOINTS",
    ),
}
ALIASES: Final = {
    "azure_ai": ("EncodingFormat",),
    "cohere": ("ChatHistory",),
    "databricks": ("AllDatabricksContentListValues", "AllDatabricksContentValues"),
    "meta": ("MuseMode", "MuseAudioEncoding", "MuseSampleRate"),
}
RESPONSE_MS_KEY: Final = "_response_ms"
VERIFY_TYPE_COUNT: Final = 3
SAMPLE_FIRST: Final = 5
SAMPLE_SECOND: Final = 2
FormT = TypeVar("FormT")


class OurSample(LiteLLMPydanticObjectBase):
    first: int = 1
    second: int | None = None


class OurResponse(betterlite_base.BaseLiteLLMOpenAIResponseObject):
    first: int = 1
    second: int = 2


def module_of(module_name: str) -> ModuleType:
    module: ModuleType = importlib.import_module(module_name)
    return module


def module_values(module_name: str) -> dict[str, JsonValue]:
    module: ModuleType = module_of(module_name)
    members: list[tuple[str, JsonValue]] = inspect.getmembers(module)
    found: dict[str, JsonValue] = dict(members)
    return found


def module_forms(module_name: str) -> dict[str, TypeForm[JsonValue]]:
    module: ModuleType = module_of(module_name)
    members: list[tuple[str, TypeForm[JsonValue]]] = inspect.getmembers(module)
    found: dict[str, TypeForm[JsonValue]] = dict(members)
    return found


def defined_classes(module_name: str) -> dict[str, type]:
    module: ModuleType = module_of(module_name)
    members: list[tuple[str, type]] = inspect.getmembers(module, inspect.isclass)
    found: dict[str, type] = {
        name: member
        for name, member in members
        if member.__module__ == module.__name__
    }
    return found


def text_list(values: list[str]) -> list[JsonValue]:
    found: list[JsonValue] = []
    found.extend(values)
    return found


def schema_of(target: TypeForm[FormT]) -> dict[str, JsonValue]:
    found: dict[str, JsonValue] = TypeAdapter(target).json_schema()
    return found


def mapping(value: JsonValue) -> dict[str, JsonValue]:
    if value is None:
        return {}
    match value:
        case dict():
            return value
        case list() | str() | bool() | int() | float():
            raise TypeError("a JSON schema object was expected")


def strings(value: JsonValue) -> list[str]:
    if value is None:
        return []
    match value:
        case list():
            found: list[str] = []
            for entry in value:
                match entry:
                    case str():
                        found.append(entry)
                    case dict() | list() | bool() | int() | float() | None:
                        raise TypeError("a JSON schema string list was expected")
            return found
        case dict() | str() | bool() | int() | float():
            raise TypeError("a JSON schema list was expected")


def schema_literals(value: JsonValue) -> list[str]:
    found: list[str] = []
    stack: list[JsonValue] = [value]
    while stack:
        current: JsonValue = stack.pop()
        match current:
            case dict():
                for key, item in current.items():
                    if key in {"const", "enum"}:
                        found.append(repr(item))
                    else:
                        stack.append(item)
            case list():
                stack.extend(current)
            case str() | bool() | int() | float() | None:
                continue
    return sorted(found)


def definition_names(schema: JsonValue) -> list[str]:
    match schema:
        case dict():
            definitions: JsonValue = schema.get("$defs")
            match definitions:
                case dict():
                    return sorted(name for name in definitions if name != "JsonValue")
                case list() | str() | bool() | int() | float() | None:
                    return []
        case list() | str() | bool() | int() | float() | None:
            return []


def typed_dict_view(cls: type) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = schema_of(cls)
    required: list[str] = strings(schema.get("required"))
    properties: dict[str, JsonValue] = mapping(schema.get("properties"))
    literals: dict[str, JsonValue] = {}
    for name, property_schema in properties.items():
        literals[name] = text_list(schema_literals(property_schema))
    view: dict[str, JsonValue] = {
        "required": text_list(required),
        "keys": text_list(sorted(properties)),
        "literals": literals,
    }
    return view


def model_view(cls: type) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = schema_of(cls)
    required: list[str] = strings(schema.get("required"))
    properties: dict[str, JsonValue] = mapping(schema.get("properties"))
    defaults: dict[str, JsonValue] = {}
    for name, property_schema in properties.items():
        defaults[name] = mapping(property_schema).get("default")
    view: dict[str, JsonValue] = {
        "required": text_list(required),
        "keys": text_list(sorted(properties)),
        "defaults": defaults,
    }
    return view


def enum_view(cls: type) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = schema_of(cls)
    members: list[str] = sorted(name for name in dir(cls) if not name.startswith("_"))
    view: dict[str, JsonValue] = {
        "values": text_list(schema_literals(schema)),
        "members": text_list(members),
    }
    return view


def alias_view(alias: TypeForm[JsonValue]) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = schema_of(alias)
    view: dict[str, JsonValue] = {
        "literals": text_list(schema_literals(schema)),
        "defs": text_list(definition_names(schema)),
    }
    return view


def class_view(cls: type) -> dict[str, JsonValue]:
    if is_typeddict(cls):
        return typed_dict_view(cls)
    if Enum in cls.__mro__:
        return enum_view(cls)
    return model_view(cls)


def dumped_of(cls: TypeForm[FormT], data: dict[str, JsonValue]) -> JsonValue:
    adapter: TypeAdapter[FormT] = TypeAdapter(cls)
    instance: FormT = adapter.validate_python(data)
    found: JsonValue = adapter.dump_python(instance)
    return found


@pytest.mark.parametrize("module_name", MODULES)
def test_names_classes(module_name: str) -> None:
    ours: dict[str, type] = defined_classes(f"betterlite.types.llms.{module_name}")
    theirs: dict[str, type] = defined_classes(f"litellm.types.llms.{module_name}")
    for name, their_class in theirs.items():
        assert name in ours, f"{module_name}.{name} is missing"
        our_class: type = ours[name]
        assert class_view(our_class) == class_view(their_class), f"{module_name}.{name}"


@pytest.mark.parametrize("module_name", sorted(CONSTANTS))
def test_names_constants(module_name: str) -> None:
    ours: dict[str, JsonValue] = module_values(f"betterlite.types.llms.{module_name}")
    theirs: dict[str, JsonValue] = module_values(f"litellm.types.llms.{module_name}")
    for name in CONSTANTS[module_name]:
        assert ours[name] == theirs[name], name


@pytest.mark.parametrize("module_name", sorted(ALIASES))
def test_names_aliases(module_name: str) -> None:
    ours: dict[str, TypeForm[JsonValue]] = module_forms(f"betterlite.types.llms.{module_name}")
    theirs: dict[str, TypeForm[JsonValue]] = module_forms(f"litellm.types.llms.{module_name}")
    for name in ALIASES[module_name]:
        assert alias_view(ours[name]) == alias_view(theirs[name]), name


def test_names_verify_types() -> None:
    ours: tuple[JsonValue, ...] = get_args(VerifyTypes)
    theirs: tuple[JsonValue, ...] = get_args(
        module_values("litellm.types.llms.custom_http")["VerifyTypes"],
    )
    assert len(ours) == VERIFY_TYPE_COUNT
    assert ours == theirs


def test_names_usage_block() -> None:
    ours: type = defined_classes("betterlite.types.llms.databricks")["ChatCompletionUsageBlock"]
    theirs: type = defined_classes("litellm.types.llms.openai")["ChatCompletionUsageBlock"]
    assert typed_dict_view(ours) == typed_dict_view(theirs)


def test_names_triton_empty() -> None:
    triton: ModuleType = module_of("betterlite.types.llms.triton")
    assert [name for name in dir(triton) if not name.startswith("_")] == []


def test_names_hidden_params() -> None:
    our_class: type = defined_classes("betterlite.types.llms.base")["HiddenParams"]
    their_class: type = defined_classes("litellm.types.llms.base")["HiddenParams"]
    ours: JsonValue = dumped_of(our_class, {"model_id": "deployment", "extra_field": 1})
    theirs: JsonValue = dumped_of(their_class, {"model_id": "deployment", "extra_field": 1})
    assert ours == theirs
    adapter: TypeAdapter[HiddenParams] = TypeAdapter(HiddenParams)
    data: dict[str, JsonValue] = {"model_id": "deployment", "extra_field": 1}
    hidden: HiddenParams = adapter.validate_python(data)
    assert hidden["extra_field"] == 1
    assert hidden.get("missing") is None
    assert hidden.model_dump()[RESPONSE_MS_KEY] is None
    our_json: JsonValue = hidden.json()
    our_dump: JsonValue = hidden.model_dump()
    assert our_json == our_dump
    with pytest.raises(AttributeError):
        _ = hidden["absent"]


def test_names_object_base_json() -> None:
    adapter: TypeAdapter[OurSample] = TypeAdapter(OurSample)
    data: dict[str, JsonValue] = {"first": 3}
    ours: OurSample = adapter.validate_python(data)
    our_json: JsonValue = ours.json()
    our_dump: JsonValue = ours.model_dump()
    assert our_json == our_dump
    without_none: JsonValue = ours.model_dump(exclude_none=True)
    assert ours.json(exclude_none=True) == without_none
    without_unset: JsonValue = ours.model_dump(exclude_unset=True)
    assert ours.json(exclude_unset=True) == without_unset
    assert ours.fields_set() == {"first"}


def test_names_response_access() -> None:
    adapter: TypeAdapter[OurResponse] = TypeAdapter(OurResponse)
    data: dict[str, JsonValue] = {"first": SAMPLE_FIRST, "extra_field": 7}
    sample: OurResponse = adapter.validate_python(data)
    assert sample["first"] == SAMPLE_FIRST
    assert sample.get("absent") is None
    assert "first" in sample
    assert "extra_field" not in sample
    items: dict[str, InitValue] = dict(sample.items())
    assert items == {"first": SAMPLE_FIRST, "second": SAMPLE_SECOND}
    with pytest.raises(KeyError):
        _ = sample["extra_field"]


def test_names_cached_tokens() -> None:
    our_class: type = defined_classes("betterlite.types.llms.base")["CachedTokensDetails"]
    their_class: type = defined_classes("litellm.types.llms.base")["CachedTokensDetails"]
    ours: JsonValue = dumped_of(our_class, {"text_tokens": 2})
    theirs: JsonValue = dumped_of(their_class, {"text_tokens": 2})
    assert ours == theirs
    assert ours == {"text_tokens": 2, "audio_tokens": None, "image_tokens": None}

import json
from collections.abc import Callable, Sized
from copy import deepcopy
from dataclasses import dataclass

import pytest
from litellm.llms.base_llm.base_utils import type_to_response_format_param as litellm_type_param  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L181
from openai.lib._pydantic import _ensure_strict_json_schema as openai_strict_schema  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L203
from pydantic import BaseModel, Field, JsonValue

from betterlite.llms.base_llm.base_utils import type_to_response_format_param

REF_TEMPLATE = "#/$defs/{model}"
URN_TEMPLATE = "urn:{model}"
ALL_OF_ENTRIES = 2
CLASS_REFERENCES = 3
DICT_FORMAT: dict[str, JsonValue] = {
    "type": "json_schema",
    "json_schema": {
        "name": "Example",
        "strict": True,
        "schema": {
            "type": "object",
            "properties": {
                "first": {"$ref": "#/$defs/Item"},
                "second": {"$ref": "#/$defs/Item"},
                "list": {"type": "array", "items": {"$ref": "#/$defs/Item"}},
            },
            "$defs": {
                "Item": {"type": "object", "properties": {"value": {"type": "integer"}}},
            },
        },
    },
}
UNSUPPORTED_MESSAGE = "Unsupported response_format type"


class Nested(BaseModel):
    value: int = Field(description="a value")


@dataclass
class PlainDataclass:
    value: int


Refused = JsonValue | BaseModel | type[PlainDataclass] | type[int] | Callable[[Sized], int]
REFUSED_VALUES: tuple[Refused, ...] = (
    42,
    "text",
    3.5,
    True,
    ["a"],
    Nested(value=1),
    PlainDataclass,
    int,
    len,
)


class Leaf(BaseModel):
    x: int


class Child(BaseModel):
    name: str
    nested: Nested
    zz: Leaf = Field(description="a leaf")


class ResponseModel(BaseModel):
    child: Child
    children: list[Child]
    optional: str | None = None
    described: Child = Field(description="a described child")


STRICT_SHAPE: dict[str, JsonValue] = {
    "type": "object",
    "default": None,
    "definitions": {"Legacy": {"type": "object", "properties": {"value": {"type": "integer"}}}},
    "$defs": {"Current": {"type": "object", "properties": {"value": {"type": "integer"}}}},
    "properties": {
        "described": {
            "type": "object",
            "description": "a single entry allOf",
            "allOf": [{"type": "object", "properties": {"a": {"type": "string"}}}],
        },
        "intersected": {
            "allOf": [
                {"type": "object", "properties": {"b": {"type": "string"}}},
                {"type": "object", "properties": {"c": {"type": "integer"}}},
            ],
        },
        "tuple_items": {"type": "array", "items": [{"type": "string"}]},
    },
}


class SchemaSource:
    @classmethod
    def model_json_schema(cls, *, ref_template: str = REF_TEMPLATE) -> dict[str, JsonValue]:
        assert ref_template == REF_TEMPLATE
        return deepcopy(STRICT_SHAPE)


def litellm_convert(
    response_format: Refused | type[BaseModel],
    ref_template: str | None = None,
) -> dict[str, JsonValue] | None:
    converted: dict[str, JsonValue] | None = litellm_type_param(response_format, ref_template)  # type: ignore[arg-type]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L179-L212
    return converted


def convert_value(value: Refused) -> dict[str, JsonValue] | None:
    converted: dict[str, JsonValue] | None = type_to_response_format_param(value)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L179-L212
    return converted


def as_mapping(value: JsonValue) -> dict[str, JsonValue]:
    match value:
        case dict():
            return value
        case bool() | int() | float() | str() | list() | None:
            raise AssertionError("the converted format is not a mapping")


def as_list(value: JsonValue) -> list[JsonValue]:
    match value:
        case list():
            return value
        case bool() | int() | float() | str() | dict() | None:
            raise AssertionError("the converted format is not a list")


def schema_of(converted: dict[str, JsonValue]) -> dict[str, JsonValue]:
    json_schema = as_mapping(converted["json_schema"])
    return as_mapping(json_schema["schema"])


def convert_same(
    response_format: type[BaseModel] | dict[str, JsonValue],
    ref_template: str | None = None,
) -> dict[str, JsonValue]:
    ours = type_to_response_format_param(response_format, ref_template)
    assert ours is not None
    assert ours == litellm_convert(response_format, ref_template)
    return ours


def test_none() -> None:
    assert type_to_response_format_param(None) is None
    assert litellm_convert(None) is None


def test_dict_passthrough() -> None:
    payload: dict[str, JsonValue] = {"type": "json_object"}
    assert type_to_response_format_param(payload) is payload
    assert litellm_convert(payload) is payload
    assert type_to_response_format_param(payload, ref_template=REF_TEMPLATE) is payload
    json_schema_payload: dict[str, JsonValue] = {"type": "json_schema", "json_schema": {}}
    assert type_to_response_format_param(json_schema_payload) is json_schema_payload


def test_dict_ref_template() -> None:
    ours = convert_same(deepcopy(DICT_FORMAT), REF_TEMPLATE)
    properties = as_mapping(schema_of(ours)["properties"])
    assert properties["first"] == {"$ref": "#/$defs/Item"}
    assert as_mapping(properties["list"])["items"] == {"$ref": "#/$defs/Item"}


def test_dict_unchanged() -> None:
    payload = deepcopy(DICT_FORMAT)
    ours = type_to_response_format_param(payload, ref_template=URN_TEMPLATE)
    reference = litellm_convert(payload, ref_template=URN_TEMPLATE)
    assert ours == reference
    assert payload == DICT_FORMAT
    assert ours is not None
    properties = as_mapping(schema_of(ours)["properties"])
    assert properties["first"] == {"$ref": "urn:Item"}


def test_dict_shared_ref() -> None:
    shared: dict[str, JsonValue] = {"$ref": "#/$defs/Item"}
    payload: dict[str, JsonValue] = {
        "type": "json_schema",
        "json_schema": {
            "schema": {"type": "object", "properties": {"a": shared, "b": shared}},
        },
    }
    ours = type_to_response_format_param(deepcopy(payload), ref_template=URN_TEMPLATE)
    reference = litellm_convert(deepcopy(payload), ref_template=URN_TEMPLATE)
    assert ours == reference
    assert ours is not None
    properties = as_mapping(schema_of(ours)["properties"])
    assert properties["a"] == {"$ref": "urn:Item"}
    assert properties["b"] == {"$ref": "urn:Item"}


def test_dict_missing_schema() -> None:
    payload: dict[str, JsonValue] = {"type": "json_schema", "json_schema": {}}
    with pytest.raises(TypeError, match=r"json_schema\.schema"):
        type_to_response_format_param(payload, ref_template=REF_TEMPLATE)
    without_schema: dict[str, JsonValue] = {"type": "json_schema"}
    with pytest.raises(TypeError, match="json_schema is not a dictionary"):
        type_to_response_format_param(without_schema, ref_template=REF_TEMPLATE)


def test_class_schema() -> None:
    ours = type_to_response_format_param(ResponseModel)
    reference = litellm_convert(ResponseModel)
    assert ours == reference
    assert ours is not None
    json_schema = as_mapping(ours["json_schema"])
    assert json_schema["name"] == "ResponseModel"
    assert json_schema["strict"] is True
    schema = as_mapping(json_schema["schema"])
    assert schema["additionalProperties"] is False
    required = sorted(str(entry) for entry in as_list(schema["required"]))
    assert required == ["child", "children", "described", "optional"]


def test_class_ref_template() -> None:
    ours = convert_same(ResponseModel, URN_TEMPLATE)
    dumped = json.dumps(schema_of(ours))
    assert "#/$defs/" not in dumped
    assert dumped.count('"urn:Child"') == CLASS_REFERENCES
    child = as_mapping(as_mapping(schema_of(ours)["properties"])["child"])
    assert child == {"$ref": "urn:Child"}


def test_strict_schema_shapes() -> None:
    converted = type_to_response_format_param(SchemaSource)
    assert converted is not None
    ours = schema_of(converted)
    # openai types root as a dict of objects, which a display takes from the argument
    reference: dict[str, JsonValue] = openai_strict_schema(
        deepcopy(STRICT_SHAPE), path=(), root={**STRICT_SHAPE},
    )
    assert ours == reference
    assert "default" not in ours
    definitions = as_mapping(ours["definitions"])
    assert as_mapping(definitions["Legacy"])["required"] == ["value"]
    assert as_mapping(as_mapping(ours["$defs"])["Current"])["additionalProperties"] is False
    properties = as_mapping(ours["properties"])
    described = as_mapping(properties["described"])
    assert "allOf" not in described
    assert described["description"] == "a single entry allOf"
    assert described["additionalProperties"] is False
    intersected = as_mapping(properties["intersected"])
    assert len(as_list(intersected["allOf"])) == ALL_OF_ENTRIES
    assert as_mapping(properties["tuple_items"])["items"] == [{"type": "string"}]


def test_unsupported() -> None:
    for value in REFUSED_VALUES:
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE):
            convert_value(value)


def test_unsupported_message() -> None:
    for value in REFUSED_VALUES:
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE) as ours_error:
            convert_value(value)
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE) as reference_error:
            litellm_convert(value)
        assert str(ours_error.value) == str(reference_error.value)

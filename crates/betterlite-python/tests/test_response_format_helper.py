from copy import deepcopy

import pytest
from litellm.llms.base_llm.base_utils import type_to_response_format_param as litellm_type_param  # pyright: ignore[reportMissingTypeStubs, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L181
from pydantic import BaseModel, Field

from betterlite.llms.base_llm.base_utils import type_to_response_format_param
from betterlite.types.utils.values import JsonValue

REF_TEMPLATE = "#/$defs/{model}"
URN_TEMPLATE = "urn:{model}"
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
INVALID_VALUES: tuple[JsonValue, ...] = (42, "text", 3.5, True, ["a"])
UNSUPPORTED_MESSAGE = "Unsupported response_format type"


class Nested(BaseModel):  # type: ignore[explicit-any]  # https://github.com/pydantic/pydantic/blob/b2212c787fb5c73386dc770681ec49cd09da8e73/pydantic/main.py#L134
    value: int = Field(description="a value")


class Child(BaseModel):  # type: ignore[explicit-any]  # https://github.com/pydantic/pydantic/blob/b2212c787fb5c73386dc770681ec49cd09da8e73/pydantic/main.py#L134
    name: str
    nested: Nested


class ResponseModel(BaseModel):  # type: ignore[explicit-any]  # https://github.com/pydantic/pydantic/blob/b2212c787fb5c73386dc770681ec49cd09da8e73/pydantic/main.py#L134
    child: Child
    children: list[Child]
    optional: str | None = None
    described: Child = Field(description="a described child")


def litellm_convert(
    response_format: type[BaseModel] | dict[str, JsonValue] | None,
    ref_template: str | None = None,
) -> dict[str, JsonValue] | None:
    converted: dict[str, JsonValue] | None = litellm_type_param(response_format, ref_template)  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L181-L212
    return converted


def litellm_convert_value(value: JsonValue) -> dict[str, JsonValue] | None:
    converted: dict[str, JsonValue] | None = litellm_type_param(value)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L181-L212
    return converted


def as_mapping(value: JsonValue) -> dict[str, JsonValue]:
    match value:
        case dict():
            return value
        case _other:
            raise AssertionError("the converted format is not a mapping")


def as_list(value: JsonValue) -> list[JsonValue]:
    match value:
        case list():
            return value
        case _other:
            raise AssertionError("the converted format is not a list")


def schema_of(converted: dict[str, JsonValue]) -> dict[str, JsonValue]:
    json_schema = as_mapping(converted["json_schema"])
    return as_mapping(json_schema["schema"])


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
    ours = type_to_response_format_param(deepcopy(DICT_FORMAT), ref_template=REF_TEMPLATE)
    reference = litellm_convert(deepcopy(DICT_FORMAT), ref_template=REF_TEMPLATE)
    assert ours == reference
    assert ours is not None
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
    ours = type_to_response_format_param(Nested, ref_template=REF_TEMPLATE)
    reference = litellm_convert(Nested, ref_template=REF_TEMPLATE)
    assert ours == reference
    assert ours is not None
    assert schema_of(ours)["type"] == "object"


def test_unsupported() -> None:
    for value in INVALID_VALUES:
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE):
            type_to_response_format_param(value)


def test_unsupported_message() -> None:
    for value in INVALID_VALUES:
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE) as ours_error:
            type_to_response_format_param(value)
        with pytest.raises(TypeError, match=UNSUPPORTED_MESSAGE) as reference_error:
            litellm_convert_value(value)
        assert str(ours_error.value) == str(reference_error.value)

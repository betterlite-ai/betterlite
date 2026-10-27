from typing import Literal, TypedDict
from unittest.mock import patch

from litellm.proxy.common_utils.custom_openapi_spec import (
    CustomOpenAPISpec,
    JsonObject,
    inline_request_body,
)


class AddSchemaToComponents(TypedDict):
    function: Literal["add_schema_to_components"]
    openapi: JsonObject
    schema_name: str
    schema: JsonObject


class AddRequestBodyToPaths(TypedDict):
    function: Literal["add_request_body_to_paths"]
    openapi: JsonObject
    paths: list[str]
    schema_ref: str


class AddRequestSchema(TypedDict):
    function: Literal["add_request_schema"]
    openapi: JsonObject
    schema_name: str
    schema: JsonObject | None
    paths: list[str]


class InlineRequestBody(TypedDict):
    function: Literal["inline_request_body"]
    schema: JsonObject | None
    example: JsonObject


Input = AddSchemaToComponents | AddRequestBodyToPaths | AddRequestSchema | InlineRequestBody


def run(input_value: Input) -> JsonObject:
    if input_value["function"] == "add_schema_to_components":
        openapi = input_value["openapi"]
        CustomOpenAPISpec.add_schema_to_components(
            openapi, input_value["schema_name"], input_value["schema"]
        )
        return openapi
    if input_value["function"] == "add_request_body_to_paths":
        openapi = input_value["openapi"]
        CustomOpenAPISpec.add_request_body_to_paths(
            openapi, input_value["paths"], input_value["schema_ref"]
        )
        return openapi
    # TypeAdapter generation is a third-party boundary, so the case gives its result or null
    with patch.object(CustomOpenAPISpec, "get_pydantic_schema", return_value=input_value["schema"]):
        if input_value["function"] == "add_request_schema":
            return CustomOpenAPISpec.add_request_schema(
                input_value["openapi"],
                object,
                input_value["schema_name"],
                input_value["paths"],
                "oracle",
            )
        return inline_request_body(object, input_value["example"])

from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class Schema(TypedDict, total=False):
    type: Literal["STRING", "INTEGER", "BOOLEAN", "NUMBER", "ARRAY", "OBJECT"]
    format: Literal["enum", "date-time"]
    title: str
    description: str
    nullable: bool
    default: JsonValue
    items: "Schema"
    minItems: str
    maxItems: str
    enum: list[str]
    properties: dict[str, "Schema"]
    propertyOrdering: list[str]
    required: list[str]
    minProperties: str
    maxProperties: str
    minimum: float
    maximum: float
    minLength: str
    maxLength: str
    pattern: str
    example: JsonValue
    anyOf: list["Schema"]


class FunctionDeclaration(TypedDict, total=False):
    name: Required[str]
    description: str
    parameters: Schema | dict[str, JsonValue]
    response: Schema

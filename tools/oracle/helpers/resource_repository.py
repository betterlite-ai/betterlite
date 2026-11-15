from typing import TypedDict

from litellm.repositories.config_repository import ConfigParam
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    raw: str


def run(input_value: Input) -> JsonValue:
    decoded: JsonValue = TypeAdapter(JsonValue).validate_json(input_value["raw"])
    param = ConfigParam("param_name", decoded)
    return TypeAdapter(JsonValue).validate_python(param.param_value)

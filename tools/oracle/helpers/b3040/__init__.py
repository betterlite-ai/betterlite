import litellm
from pydantic import JsonValue, TypeAdapter

from .operations import (
    descriptor_answer,
    dynamic_flag_answer,
    enforce_answer,
    limit_answer,
)
from .types import Input

litellm.suppress_debug_info = True

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "dynamic_flag":
        return JSON.validate_python(dynamic_flag_answer(input_value))
    if input_value["function"] == "should_enforce":
        return JSON.validate_python(enforce_answer(input_value))
    if input_value["function"] == "enforced_limit":
        return JSON.validate_python(limit_answer(input_value))
    return JSON.validate_python(descriptor_answer(input_value))

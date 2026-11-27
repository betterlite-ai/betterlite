from pydantic import JsonValue

from .engine import dispatch_value, mapping_value, request_answer, transform_value
from .inputs import Input


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "mapping":
        return mapping_value(input_value)
    if input_value["operation"] == "request":
        return request_answer(input_value)
    if input_value["operation"] == "dispatch":
        return dispatch_value(input_value)
    return transform_value(input_value)

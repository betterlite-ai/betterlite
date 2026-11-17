from typing import TypedDict

import litellm
from pydantic import JsonValue

from studio_image_edit.edit_config import factory_value
from studio_image_edit.edit_cost import CostCase, cost_value
from studio_image_edit.edit_request import RequestCase, request_value
from studio_image_edit.edit_response import ResponseCase, response_value
from studio_image_edit.edit_values import JSON, JsonObject


class Input(TypedDict):
    factories: list[str]
    requests: list[RequestCase]
    responses: list[ResponseCase]
    costs: list[CostCase]


def run(input_value: Input) -> JsonValue:
    # an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
    litellm.suppress_debug_info = True
    answer: JsonObject = {
        "factories": [factory_value(model) for model in input_value["factories"]],
        "requests": [request_value(case) for case in input_value["requests"]],
        "responses": [response_value(case) for case in input_value["responses"]],
        "costs": [cost_value(case) for case in input_value["costs"]],
    }
    return JSON.validate_python(answer)

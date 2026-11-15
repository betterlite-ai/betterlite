from decimal import Decimal
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.llms.fal_ai.cost_calculator import cost_calculator, fal_ai_passthrough_cost
from litellm.types.utils import ImageObject, ImageResponse, ModelInfo
from pydantic import JsonValue

CALCULATOR_OPERATIONS: TypeAlias = Literal[
    "deployment_zero_wins",
    "resolution_case_sensitive",
    "area_tie_smaller",
    "returned_dimensions_win",
    "megapixel_round_up",
    "empty_response_no_lookup",
]


class CalculatorCase(TypedDict):
    images: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    model_info: ModelInfo | None


class PassthroughCase(TypedDict):
    model: str
    request_body: dict[str, JsonValue]


class CalculatorInput(TypedDict):
    operation: CALCULATOR_OPERATIONS
    model: str
    catalog: dict[str, ModelInfo]
    cases: list[CalculatorCase]
    passthrough_cases: list[PassthroughCase]


class PassthroughInput(TypedDict):
    operation: Literal["passthrough_count_types"]
    model: str
    catalog: dict[str, ModelInfo]
    cases: list[CalculatorCase]
    passthrough_cases: list[PassthroughCase]


Input: TypeAlias = CalculatorInput | PassthroughInput


def image_response(images: list[dict[str, JsonValue]]) -> ImageResponse:
    return ImageResponse(
        data=[ImageObject(provider_specific_fields=fields) for fields in images]
    )


def calculator_costs(call: CalculatorInput) -> JsonValue:
    litellm.model_cost = dict(call["catalog"])
    costs: list[JsonValue] = []
    for case in call["cases"]:
        cost = cost_calculator(
            model=call["model"],
            image_response=image_response(case["images"]),
            optional_params=dict(case["optional_params"]),
            model_info=case["model_info"],
        )
        costs.append(format(Decimal(repr(cost)).normalize(), "f"))
    return {"costs": costs}


def passthrough_costs(call: PassthroughInput) -> JsonValue:
    litellm.model_cost = dict(call["catalog"])
    costs: list[JsonValue] = []
    for case in call["passthrough_cases"]:
        cost = fal_ai_passthrough_cost(case["model"], dict(case["request_body"]))
        costs.append(
            None if cost is None else format(Decimal(repr(cost)).normalize(), "f")
        )
    return {"costs": costs}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "passthrough_count_types":
        return passthrough_costs(input_value)
    return calculator_costs(input_value)

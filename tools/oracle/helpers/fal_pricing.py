from decimal import Decimal
from typing import Literal, TypedDict, cast

import litellm
from litellm.llms.fal_ai.cost_calculator import cost_calculator, fal_ai_passthrough_cost
from litellm.types.utils import ImageObject, ImageResponse, ModelInfo
from pydantic import JsonValue


class CalculatorCase(TypedDict):
    images: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue] | None


class PassthroughCase(TypedDict):
    model: str
    request_body: dict[str, JsonValue]


class Input(TypedDict):
    operation: Literal[
        "deployment_zero_wins",
        "resolution_case_sensitive",
        "area_tie_smaller",
        "returned_dimensions_win",
        "megapixel_round_up",
        "empty_response_no_lookup",
        "passthrough_count_types",
    ]
    model: str
    catalog: dict[str, dict[str, JsonValue]]
    cases: list[CalculatorCase]
    passthrough_cases: list[PassthroughCase]


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = dict(input_value["catalog"])
    costs: list[JsonValue] = []
    if input_value["operation"] == "passthrough_count_types":
        for passthrough in input_value["passthrough_cases"]:
            price = fal_ai_passthrough_cost(
                passthrough["model"], dict(passthrough["request_body"])
            )
            costs.append(
                None if price is None else format(Decimal(repr(price)).normalize(), "f")
            )
        return {"costs": costs}
    for case in input_value["cases"]:
        response = ImageResponse(
            data=[
                ImageObject(provider_specific_fields=fields) for fields in case["images"]
            ]
        )
        cost = cost_calculator(
            model=input_value["model"],
            image_response=response,
            optional_params=dict(case["optional_params"]),
            model_info=cast("ModelInfo | None", case["model_info"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
        )
        costs.append(format(Decimal(repr(cost)).normalize(), "f"))
    return {"costs": costs}

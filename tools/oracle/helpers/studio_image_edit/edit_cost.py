from decimal import Decimal
from typing import NotRequired, TypedDict

import litellm
from litellm.llms.gemini.image_edit.cost_calculator import cost_calculator as edit_cost_calculator
from litellm.types.utils import ImageObject, ImageResponse, ImageUsage
from pydantic import JsonValue

from studio_image_edit.edit_values import JsonObject


class CostUsage(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    text_tokens: NotRequired[int]
    image_tokens: NotRequired[int]


class CostCase(TypedDict):
    model: str
    catalog: JsonObject
    images: int
    usage: NotRequired[CostUsage]


def cost_usage_fields(usage: CostUsage) -> JsonObject:
    image_tokens = usage.get("image_tokens")
    text_tokens = usage.get("text_tokens")
    return {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": {
            "image_tokens": 0 if image_tokens is None else image_tokens,
            "text_tokens": 0 if text_tokens is None else text_tokens,
        },
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def cost_value(case: CostCase) -> JsonObject:
    entry: JsonObject = {
        **case["catalog"],
        "litellm_provider": "gemini",
        "mode": "image_generation",
    }
    pricing: dict[str, dict[str, JsonValue]] = {f"gemini/{case['model']}": entry}
    litellm.register_model(pricing)
    response = ImageResponse(
        data=[ImageObject(b64_json="AAA") for _ in range(case["images"])]
    )
    usage = case.get("usage")
    if usage is None:
        response.usage = None
    else:
        response.usage = ImageUsage.model_validate(cost_usage_fields(usage))
    cost = edit_cost_calculator(case["model"], response)
    return {"cost": format(Decimal(repr(cost)).normalize(), "f")}

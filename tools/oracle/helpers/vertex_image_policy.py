from decimal import Decimal
from typing import Final, Literal, TypedDict

import litellm
from litellm.llms.vertex_ai.image_edit import get_vertex_ai_image_edit_config
from litellm.llms.vertex_ai.image_edit.cost_calculator import cost_calculator as edit_cost
from litellm.llms.vertex_ai.image_generation import get_vertex_ai_image_generation_config
from litellm.llms.vertex_ai.image_generation.cost_calculator import (
    cost_calculator as generation_cost,
)
from litellm.llms.vertex_ai.image_generation.image_generation_handler import (
    VertexImageGeneration,
)
from litellm.types.utils import ImageObject, ImageResponse, ImageUsage
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HANDLER: Final = VertexImageGeneration()


class ImageDatum(TypedDict):
    b64_json: str | None


class ImageReply(TypedDict):
    data: list[ImageDatum]


REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)


class UsageCase(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    web_search_requests: int | None


class ImagesCase(TypedDict):
    images: int
    usage: UsageCase | None


class BillingCase(TypedDict):
    kind: Literal["generation", "edit"]
    model: str
    catalog: JsonObject
    response: ImagesCase | None


class ReplyCase(TypedDict):
    kind: Literal["detect", "convert"]
    body: JsonObject
    existing: list[str | None]


class Input(TypedDict):
    models: list[str]
    billing: list[BillingCase]
    replies: list[ReplyCase]


def model_value(model: str) -> JsonObject:
    generation = get_vertex_ai_image_generation_config(model)
    edit = get_vertex_ai_image_edit_config(model)
    return {
        "model": model,
        "generation": type(generation).__name__,
        "edit": type(edit).__name__,
    }


def image_response(case: ImagesCase) -> ImageResponse:
    response = ImageResponse(data=[ImageObject(b64_json="AAA") for _ in range(case["images"])])
    usage = case["usage"]
    if usage is None:
        # ImageResponse() builds a zero usage when given none, so a missing one is set after
        response.usage = None
        return response
    fields: JsonObject = {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": {"image_tokens": 0, "text_tokens": 0},
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }
    if usage["web_search_requests"] is not None:
        fields["web_search_requests"] = usage["web_search_requests"]
    response.usage = ImageUsage.model_validate(fields)
    return response


def billing_value(case: BillingCase) -> JsonObject:
    entry: JsonObject = {
        **case["catalog"],
        "litellm_provider": "vertex_ai",
        "mode": "image_generation",
    }
    litellm.register_model({f"vertex_ai/{case['model']}": entry})
    response = case["response"]
    if case["kind"] == "edit":
        # anything but an ImageResponse is what the null response stands for
        cost = edit_cost(case["model"], {} if response is None else image_response(response))
    elif response is None:
        raise ValueError("a generation cost needs a response")
    else:
        cost = generation_cost(case["model"], image_response(response))
    return {"cost": format(Decimal(repr(cost)).normalize(), "f")}


def reply_value(case: ReplyCase) -> JsonObject:
    if case["kind"] == "detect":
        return {"detected": HANDLER.is_image_generation_response(case["body"])}
    held = ImageResponse(data=[ImageObject(b64_json=encoded) for encoded in case["existing"]])
    finished = HANDLER.process_image_generation_response(case["body"], held)
    parsed = REPLY.validate_json(finished.model_dump_json())
    images: list[JsonValue] = [datum["b64_json"] for datum in parsed["data"]]
    return {"images": images}


def run(input_value: Input) -> JsonValue:
    models: list[JsonValue] = [model_value(model) for model in input_value["models"]]
    billing: list[JsonValue] = [billing_value(case) for case in input_value["billing"]]
    replies: list[JsonValue] = [reply_value(case) for case in input_value["replies"]]
    answer: JsonObject = {"models": models, "billing": billing, "replies": replies}
    return JSON.validate_python(answer)

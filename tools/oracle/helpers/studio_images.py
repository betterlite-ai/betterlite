import json
import os
from collections.abc import Callable
from decimal import Decimal
from typing import Final, Never, NotRequired, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.gemini import image_generation as gemini_image_generation
from litellm.llms.gemini.image_generation.cost_calculator import cost_calculator
from litellm.llms.gemini.image_generation.transformation import GoogleImageGenConfig
from litellm.types.utils import ImageObject, ImageResponse, ImageUsage
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
STUDIO_VARIABLES: Final = ("GEMINI_API_KEY", "GEMINI_API_BASE", "GOOGLE_API_KEY")


@runtime_checkable
class StudioImageConfig(Protocol):
    map_openai_params: Callable[
        [dict[str, JsonValue], dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    get_complete_url: Callable[
        [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue], bool | None],
        str,
    ]
    validate_environment: Callable[
        [
            dict[str, str],
            str,
            list[Never],
            dict[str, JsonValue],
            dict[str, JsonValue],
            str | None,
            str | None,
        ],
        dict[str, JsonValue],
    ]
    # ast-grep-ignore: no-long-names
    transform_image_generation_request: Callable[
        [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, JsonValue]],
        dict[str, JsonValue],
    ]
    # ast-grep-ignore: no-long-names
    transform_image_generation_response: Callable[
        [
            str,
            httpx.Response,
            ImageResponse,
            None,
            dict[str, JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            None,
        ],
        ImageResponse,
    ]


# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[StudioImageConfig] = TypeAdapter[StudioImageConfig](
    StudioImageConfig, config=ARBITRARY_TYPES
).validate_python(GoogleImageGenConfig())


class RequestCase(TypedDict):
    model: str
    prompt: str
    params: NotRequired[JsonObject]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    image_size_support: NotRequired[bool | None]
    env: NotRequired[dict[str, str]]


class ResponseCase(TypedDict):
    model: str
    body: JsonObject


class CostUsage(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    text_tokens: NotRequired[int]
    image_tokens: NotRequired[int]
    web_search_requests: NotRequired[int]


class CostCase(TypedDict):
    model: str
    catalog: JsonObject
    images: int
    usage: NotRequired[CostUsage]


class Input(TypedDict):
    factories: list[str]
    requests: list[RequestCase]
    responses: list[ResponseCase]
    costs: list[CostCase]


class ImageDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class TokenDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class UsageFields(TypedDict):
    input_tokens: int
    input_tokens_details: TokenDetails
    output_tokens: int
    total_tokens: int
    prompt_tokens: int
    prompt_tokens_details: TokenDetails
    completion_tokens: int
    completion_tokens_details: TokenDetails
    output_tokens_details: TokenDetails
    web_search_requests: NotRequired[int]


class ImageReply(TypedDict):
    data: list[ImageDatum]
    usage: UsageFields | None


REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)


def apply_environment(env: dict[str, str] | None) -> None:
    # only the three Studio variables change, so dispatch's cost-map setting stays in place
    for name in STUDIO_VARIABLES:
        os.environ.pop(name, None)
    if env is not None:
        os.environ.update(env)


def factory_value(model: str) -> JsonObject:
    config = gemini_image_generation.get_gemini_image_generation_config(model)
    return {"model": model, "config": type(config).__name__}


def request_value(case: RequestCase) -> JsonObject:
    apply_environment(case.get("env"))
    model = case["model"]
    prompt = case["prompt"]
    given_params = case.get("params")
    params: JsonObject = {} if given_params is None else given_params
    given_headers = case.get("headers")
    headers: dict[str, str] = {} if given_headers is None else given_headers
    api_key = case.get("api_key")
    api_base = case.get("api_base")
    mapped = CONFIG.map_openai_params(params, {}, model, DROP_PARAMS)
    url = CONFIG.get_complete_url(api_base, api_key, model, {}, {}, None)
    outgoing = CONFIG.validate_environment(headers, model, [], {}, {}, api_key, api_base)
    body = CONFIG.transform_image_generation_request(model, prompt, mapped, {}, outgoing)
    return {"url": url, "headers": outgoing, "body": body}


def detail_value(details: TokenDetails) -> JsonObject:
    return {
        "image_tokens": details["image_tokens"],
        "text_tokens": details["text_tokens"],
    }


def usage_value(usage: UsageFields) -> JsonObject:
    return {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": detail_value(usage["input_tokens_details"]),
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
        "prompt_tokens": usage["prompt_tokens"],
        "prompt_tokens_details": detail_value(usage["prompt_tokens_details"]),
        "completion_tokens": usage["completion_tokens"],
        "completion_tokens_details": detail_value(usage["completion_tokens_details"]),
        "output_tokens_details": detail_value(usage["output_tokens_details"]),
        "web_search_requests": usage.get("web_search_requests"),
    }


def image_value(datum: ImageDatum) -> JsonObject:
    fields = datum["provider_specific_fields"]
    signature = None if fields is None else fields.get("thought_signature")
    return {"b64_json": datum["b64_json"], "thought_signature": signature}


def response_value(case: ResponseCase) -> JsonObject:
    model_response = ImageResponse()
    # the reply alone decides whether usage exists, so no default zero usage is seeded here
    model_response.usage = None
    raw = httpx.Response(200, content=json.dumps(case["body"]))
    finished = CONFIG.transform_image_generation_response(
        case["model"], raw, model_response, None, {}, {}, {}, None
    )
    parsed = REPLY.validate_json(finished.model_dump_json())
    usage = parsed["usage"]
    return {
        "images": [image_value(datum) for datum in parsed["data"]],
        "usage": None if usage is None else usage_value(usage),
    }


def cost_usage_fields(usage: CostUsage) -> JsonObject:
    image_tokens = usage.get("image_tokens")
    text_tokens = usage.get("text_tokens")
    fields: JsonObject = {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": {
            "image_tokens": 0 if image_tokens is None else image_tokens,
            "text_tokens": 0 if text_tokens is None else text_tokens,
        },
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }
    searched = usage.get("web_search_requests")
    if searched is not None:
        fields["web_search_requests"] = searched
    return fields


def cost_value(case: CostCase) -> JsonObject:
    entry: JsonObject = {
        **case["catalog"],
        "litellm_provider": "gemini",
        "mode": "image_generation",
    }
    pricing: dict[str, dict[str, JsonValue]] = {f"gemini/{case['model']}": entry}
    litellm.register_model(pricing)
    response = ImageResponse(data=[ImageObject(b64_json="AAA") for _ in range(case["images"])])
    usage = case.get("usage")
    if usage is None:
        response.usage = None
    else:
        response.usage = ImageUsage.model_validate(cost_usage_fields(usage))
    cost = cost_calculator(case["model"], response)
    return {"cost": format(Decimal(repr(cost)).normalize(), "f")}


def run(input_value: Input) -> JsonValue:
    answer: JsonObject = {
        "factories": [factory_value(model) for model in input_value["factories"]],
        "requests": [request_value(case) for case in input_value["requests"]],
        "responses": [response_value(case) for case in input_value["responses"]],
        "costs": [cost_value(case) for case in input_value["costs"]],
    }
    return JSON.validate_python(answer)

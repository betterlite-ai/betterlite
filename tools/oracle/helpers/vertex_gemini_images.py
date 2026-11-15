import json
import logging
import os
from collections.abc import Callable
from typing import Final, Never, NotRequired, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.gemini.common_utils import get_gemini_image_web_search_requests
from litellm.llms.vertex_ai.image_generation.vertex_gemini_transformation import (
    VertexAIGeminiImageGenerationConfig,
)
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
LOGGER: Final = logging.getLogger("LiteLLM")
VERTEX_VARIABLES: Final = (
    "VERTEXAI_PROJECT",
    "VERTEXAI_LOCATION",
    "VERTEX_LOCATION",
    "VERTEXAI_CREDENTIALS",
    "GOOGLE_APPLICATION_CREDENTIALS",
)


@runtime_checkable
class GeminiImageConfig(Protocol):
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
    transform_image_generation_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
        dict[str, JsonValue],
    ]
    transform_image_generation_response: Callable[  # ast-grep-ignore: no-long-names
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
CONFIG: Final[GeminiImageConfig] = TypeAdapter[GeminiImageConfig](
    GeminiImageConfig, config=ARBITRARY_TYPES
).validate_python(VertexAIGeminiImageGenerationConfig())


class ImageDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class TokenDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class ImageUsageFields(TypedDict):
    input_tokens: int
    input_tokens_details: TokenDetails
    output_tokens: int
    total_tokens: int
    web_search_requests: NotRequired[int]


class ImageReply(TypedDict):
    data: list[ImageDatum]
    usage: ImageUsageFields | None


REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)


class MappingCase(TypedDict):
    prompt: str
    non_default_params: JsonObject
    optional_params: JsonObject


class TargetCase(TypedDict):
    api_base: str | None
    params_api_base: str | None
    project: str | None
    location: str | None
    headers: dict[str, str]


class Input(TypedDict):
    model: str
    mappings: list[MappingCase]
    target: TargetCase | None
    response: JsonObject | None


class WarningCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def mapping_value(model: str, case: MappingCase) -> JsonObject:
    counter = WarningCounter()
    LOGGER.addHandler(counter)
    try:
        mapped = CONFIG.map_openai_params(
            case["non_default_params"], case["optional_params"], model, DROP_PARAMS
        )
    finally:
        LOGGER.removeHandler(counter)
    body = CONFIG.transform_image_generation_request(
        model, case["prompt"], mapped, {}, {}
    )
    return {"mapped": mapped, "warnings": len(counter.messages), "body": body}


def set_environment(target: TargetCase) -> None:
    for name in VERTEX_VARIABLES:
        os.environ.pop(name, None)
    project = target["project"]
    if project is not None:
        os.environ["VERTEXAI_PROJECT"] = project
    location = target["location"]
    if location is not None:
        os.environ["VERTEXAI_LOCATION"] = location


def target_value(model: str, target: TargetCase) -> JsonObject:
    set_environment(target)
    api_base = target["api_base"]
    params: JsonObject = {}
    if target["params_api_base"] is not None:
        params["api_base"] = target["params_api_base"]
    url = CONFIG.get_complete_url(api_base, None, model, {}, params, None)
    headers = CONFIG.validate_environment(
        dict(target["headers"]), model, [], {}, params, None, api_base
    )
    return {"url": url, "headers": headers}


def datum_value(datum: ImageDatum) -> JsonObject:
    entry: JsonObject = {}
    if datum["b64_json"] is not None:
        entry["b64_json"] = datum["b64_json"]
    if datum["provider_specific_fields"] is not None:
        entry["provider_specific_fields"] = datum["provider_specific_fields"]
    return entry


def usage_value(usage: ImageUsageFields) -> JsonObject:
    details = usage["input_tokens_details"]
    value: JsonObject = {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": JSON.validate_python(
            {"image_tokens": details["image_tokens"], "text_tokens": details["text_tokens"]}
        ),
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }
    searched = usage.get("web_search_requests")
    if searched is not None:
        value["web_search_requests"] = searched
    return value


def reply_value(model: str, reply: JsonObject) -> JsonObject:
    model_response = ImageResponse()
    # the reply alone decides whether usage exists, so no default zero usage is seeded here
    model_response.usage = None
    raw = httpx.Response(200, content=json.dumps(reply))
    finished = CONFIG.transform_image_generation_response(
        model, raw, model_response, None, {}, {}, {}, None
    )
    parsed = REPLY.validate_json(finished.model_dump_json())
    images: list[JsonValue] = [datum_value(datum) for datum in parsed["data"]]
    response: JsonObject = {"data": images}
    usage = parsed["usage"]
    if usage is not None:
        response["usage"] = usage_value(usage)
    return {
        "response": response,
        "search_requests": get_gemini_image_web_search_requests(reply),
    }


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    mappings: list[JsonValue] = [
        mapping_value(model, case) for case in input_value["mappings"]
    ]
    target = input_value["target"]
    response = input_value["response"]
    answer: JsonObject = {
        "mappings": mappings,
        "target": None if target is None else target_value(model, target),
        "response": None if response is None else reply_value(model, response),
    }
    return JSON.validate_python(answer)

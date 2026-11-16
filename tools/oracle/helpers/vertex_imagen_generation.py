import json
import os
from collections.abc import Callable
from typing import Final, Never, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.vertex_ai.image_generation.vertex_imagen_transformation import (
    VertexAIImagenImageGenerationConfig,
)
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False
VERTEX_VARIABLES: Final = (
    "VERTEXAI_PROJECT",
    "VERTEXAI_LOCATION",
    "VERTEX_LOCATION",
    "VERTEXAI_CREDENTIALS",
    "GOOGLE_APPLICATION_CREDENTIALS",
)


@runtime_checkable
class ImagenImageConfig(Protocol):
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
CONFIG: Final[ImagenImageConfig] = TypeAdapter[ImagenImageConfig](
    ImagenImageConfig, config=ARBITRARY_TYPES
).validate_python(VertexAIImagenImageGenerationConfig())


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


class ImageReply(TypedDict):
    data: list[ImageDatum]
    usage: ImageUsageFields | None


REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)


class MappingCase(TypedDict):
    prompt: str
    non_default_params: JsonObject
    optional_params: JsonObject
    litellm_params: JsonObject | None


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


def set_environment(target: TargetCase) -> None:
    for name in VERTEX_VARIABLES:
        os.environ.pop(name, None)
    chosen: dict[str, str | None] = {
        "VERTEXAI_PROJECT": target["project"],
        "VERTEXAI_LOCATION": target["location"],
    }
    for name, value in chosen.items():
        if value is not None:
            os.environ[name] = value


def mapping_value(model: str, case: MappingCase) -> JsonObject:
    mapped = CONFIG.map_openai_params(
        case["non_default_params"], case["optional_params"], model, DROP_PARAMS
    )
    litellm_params = case["litellm_params"]
    body = CONFIG.transform_image_generation_request(
        model, case["prompt"], mapped, {} if litellm_params is None else litellm_params, {}
    )
    return {"mapped": mapped, "body": body}


def target_value(model: str, target: TargetCase) -> JsonObject:
    set_environment(target)
    api_base = target["api_base"]
    params: JsonObject = {}
    if target["params_api_base"] is not None:
        params["api_base"] = target["params_api_base"]
    url = CONFIG.get_complete_url(api_base, None, model, {}, params, None)
    resolved = CONFIG.validate_environment(
        dict(target["headers"]), model, [], {}, params, None, api_base
    )
    return {"url": url, "headers": resolved}


def datum_value(datum: ImageDatum) -> JsonObject:
    encoded = datum["b64_json"]
    return {} if encoded is None else {"b64_json": encoded}


def usage_value(usage: ImageUsageFields) -> JsonObject:
    details = usage["input_tokens_details"]
    input_details: JsonObject = {
        "image_tokens": details["image_tokens"],
        "text_tokens": details["text_tokens"],
    }
    return {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": input_details,
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def reply_value(model: str, reply: JsonObject) -> JsonObject:
    model_response = ImageResponse()
    raw = httpx.Response(200, content=json.dumps(reply))
    # ImageResponse() fills in a zero usage; the transform leaves usage exactly as it found it
    finished = CONFIG.transform_image_generation_response(
        model, raw, model_response, None, {}, {}, {}, None
    )
    parsed = REPLY.validate_json(finished.model_dump_json())
    images: list[JsonValue] = [datum_value(datum) for datum in parsed["data"]]
    response: JsonObject = {"data": images}
    usage = parsed["usage"]
    if usage is not None:
        response["usage"] = usage_value(usage)
    return {"response": response}


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    mapped: list[JsonValue] = [mapping_value(model, case) for case in input_value["mappings"]]
    target = input_value["target"]
    reply = input_value["response"]
    answer: JsonObject = {
        "mappings": mapped,
        "target": None if target is None else target_value(model, target),
        "response": None if reply is None else reply_value(model, reply),
    }
    return JSON.validate_python(answer)

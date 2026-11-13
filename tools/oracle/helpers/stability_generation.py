from collections.abc import Callable
from typing import Literal, Protocol, TypedDict, cast

import httpx
from litellm.llms.stability.image_generation import get_stability_image_generation_config
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.image import Image
from pydantic import JsonValue

MODEL = "stability/sd3"
EMPTY: dict[str, JsonValue] = {}


class SupportedCall(TypedDict):
    function: Literal["supported"]
    model: str


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class UrlCall(TypedDict):
    function: Literal["url"]
    model: str
    api_base: str | None
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    model: str
    api_key: str | None
    headers: dict[str, str]
    environment: dict[str, str]


class RequestCall(TypedDict):
    function: Literal["request"]
    prompt: str
    optional_params: dict[str, JsonValue]


class ImageSeed(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None


class ResponseCall(TypedDict):
    function: Literal["response"]
    status: int
    headers: dict[str, str]
    body: str
    created: int
    data: list[ImageSeed]


Call = SupportedCall | MapCall | UrlCall | EnvironmentCall | RequestCall | ResponseCall


class Input(TypedDict):
    calls: list[Call]


class GenerationConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
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
            list[JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            str | None,
        ],
        dict[str, JsonValue],
    ]
    transform_image_generation_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, JsonValue]],
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


def load_config(model: str) -> GenerationConfig:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/stability/image_generation/transformation.py#L95-L154
        "GenerationConfig", get_stability_image_generation_config(model)
    )


def supported_call(call: SupportedCall) -> JsonValue:
    config = load_config(call["model"])
    supported = config.get_supported_openai_params(call["model"])
    return [str(name) for name in supported]


def map_call(call: MapCall) -> JsonValue:
    config = load_config(call["model"])
    try:
        mapped = config.map_openai_params(
            call["non_default_params"],
            call["optional_params"],
            call["model"],
            call["drop_params"],
        )
    except ValueError:
        return {"unsupported": True}
    return {"mapped": mapped}


def url_call(call: UrlCall) -> JsonValue:
    config = load_config(call["model"])
    url = config.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY, None)
    return {"url": url}


def environment_call(call: EnvironmentCall) -> JsonValue:
    config = load_config(call["model"])
    headers: dict[str, str] = dict(call["headers"])
    try:
        validated = config.validate_environment(
            headers, call["model"], [], EMPTY, EMPTY, call["api_key"]
        )
    except ValueError:
        return {"missing_key": True}
    return {"headers": validated}


def request_call(call: RequestCall) -> JsonValue:
    config = load_config(MODEL)
    request = config.transform_image_generation_request(
        MODEL, call["prompt"], call["optional_params"], EMPTY, EMPTY
    )
    return {"request": request}


def seeded_images(seeds: list[ImageSeed]) -> list[Image]:
    images: list[Image] = [
        ImageObject(
            b64_json=seed["b64_json"],
            url=seed["url"],
            revised_prompt=seed["revised_prompt"],
        )
        for seed in seeds
    ]
    return images


def response_value(response: ImageResponse) -> JsonValue:
    data = response.data
    if data is None:
        return {"created": response.created, "data": []}
    images: list[JsonValue] = []
    for image in data:
        entry: dict[str, JsonValue] = {}
        if image.b64_json is not None:
            entry["b64_json"] = image.b64_json
        if image.url is not None:
            entry["url"] = image.url
        if image.revised_prompt is not None:
            entry["revised_prompt"] = image.revised_prompt
        images.append(entry)
    return {"created": response.created, "data": images}


def response_call(call: ResponseCall) -> JsonValue:
    config = load_config(MODEL)
    model_response = ImageResponse(created=call["created"])
    if call["data"]:
        model_response.data = seeded_images(call["data"])
    finished = config.transform_image_generation_response(
        MODEL,
        httpx.Response(call["status"], headers=call["headers"], content=call["body"]),
        model_response,
        None,
        EMPTY,
        EMPTY,
        EMPTY,
        None,
    )
    return {"response": response_value(finished)}


def call_value(call: Call) -> JsonValue:
    match call["function"]:
        case "supported":
            return supported_call(call)
        case "map":
            return map_call(call)
        case "url":
            return url_call(call)
        case "environment":
            return environment_call(call)
        case "request":
            return request_call(call)
        case "response":
            return response_call(call)


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    return [call_value(call) for call in calls]

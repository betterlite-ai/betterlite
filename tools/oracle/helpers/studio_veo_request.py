import base64
import io
import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, runtime_checkable

import litellm
from litellm.llms.gemini.videos.transformation import GeminiVideoConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
Headers = dict[str, str]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False


@runtime_checkable
class VideoConfig(Protocol):
    map_openai_params: Callable[
        [dict[str, JsonValue | io.BytesIO], str, bool],
        dict[str, JsonValue | io.BytesIO],
    ]
    validate_environment: Callable[
        [dict[str, str], str, str | None, GenericLiteLLMParams],
        dict[str, str],
    ]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    transform_video_create_request: Callable[
        [
            str,
            str,
            str,
            dict[str, JsonValue | io.BytesIO],
            GenericLiteLLMParams,
            dict[str, str],
        ],
        tuple[JsonObject, JsonValue, str],
    ]


# GeminiVideoConfig.__init__ is untyped, and every method this file calls keeps no state
HANDLER: Final = GeminiVideoConfig.__new__(GeminiVideoConfig)
# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[VideoConfig] = TypeAdapter[VideoConfig](
    VideoConfig, config=ARBITRARY_TYPES
).validate_python(HANDLER)


class ImageSeed(TypedDict):
    kind: Literal["dict", "file", "null", "text"]
    value: NotRequired[JsonObject | str]
    data: NotRequired[str]
    position: NotRequired[int]


class Request(TypedDict):
    model: str
    prompt: str
    api_base: NotRequired[str | None]
    input_reference: NotRequired[ImageSeed]
    image: NotRequired[ImageSeed]
    params: NotRequired[JsonObject]
    env: NotRequired[Headers]
    generic_key: NotRequired[str | None]
    api_key: NotRequired[str | None]
    params_api_key: NotRequired[str | None]
    headers: NotRequired[Headers]


class Prepared(TypedDict):
    params: dict[str, JsonValue | io.BytesIO]
    reference: io.BytesIO | None
    image: io.BytesIO | None


class Input(TypedDict):
    requests: list[Request]


def seed_reader(seed: ImageSeed | None) -> io.BytesIO | None:
    if seed is None or seed["kind"] != "file":
        return None
    if "data" not in seed:
        raise ValueError("a file image seed carries its data")
    buffer = io.BytesIO(base64.b64decode(seed["data"]))
    position = seed.get("position")
    buffer.seek(0 if position is None else position)
    return buffer


def image_value(seed: ImageSeed, reader: io.BytesIO | None) -> JsonValue | io.BytesIO:
    kind = seed["kind"]
    if kind == "file":
        return reader
    if kind == "null":
        return None
    if "value" not in seed:
        raise ValueError("a dict or text image seed carries its value")
    return seed["value"]


def prepare(request: Request) -> Prepared:
    reference = seed_reader(request.get("input_reference"))
    image = seed_reader(request.get("image"))
    params: dict[str, JsonValue | io.BytesIO] = {}
    given_reference = request.get("input_reference")
    if given_reference is not None:
        params["input_reference"] = image_value(given_reference, reference)
    given_image = request.get("image")
    if given_image is not None:
        params["image"] = image_value(given_image, image)
    given = request.get("params")
    if given is not None:
        for name, value in given.items():
            params[name] = value
    return {"params": params, "reference": reference, "image": image}


def apply_environment(request: Request) -> None:
    given = request.get("env")
    os.environ.clear()
    if given is not None:
        os.environ.update(given)
    litellm.api_key = request.get("generic_key")


def chosen_position(request: Request, prepared: Prepared) -> int | None:
    if request.get("input_reference") is not None:
        chosen = prepared["reference"]
    else:
        chosen = prepared["image"]
    if chosen is None:
        return None
    return chosen.tell()


def answer(request: Request) -> JsonValue:
    apply_environment(request)
    prepared = prepare(request)
    model = request["model"]
    litellm_params = GenericLiteLLMParams(api_key=request.get("params_api_key"))
    mapped = CONFIG.map_openai_params(prepared["params"], model, DROP_PARAMS)
    url = CONFIG.get_complete_url(model, request.get("api_base"), {})
    given = request.get("headers")
    headers: Headers = {} if given is None else dict(given)
    outgoing = CONFIG.validate_environment(headers, model, request.get("api_key"), litellm_params)
    body, _files, returned = CONFIG.transform_video_create_request(
        model, request["prompt"], url, mapped, litellm_params, outgoing
    )
    value: JsonObject = {"url": returned, "headers": JsonObject(outgoing), "body": body}
    position = chosen_position(request, prepared)
    if position is not None:
        value["image_position"] = position
    return JSON.validate_python(value)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(request) for request in input_value["requests"]]
    value: JsonObject = {"requests": answers}
    return JSON.validate_python(value)

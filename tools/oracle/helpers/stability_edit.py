import base64
import os
import time
from collections.abc import Callable
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
import typing_extensions
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.stability.image_edit import get_stability_image_edit_config as edit_config
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
ENVIRONMENT: Final[tuple[str, ...]] = ("STABILITY_API_KEY", "STABILITY_API_BASE")
NO_HEADERS: Final[dict[str, JsonValue]] = {}


# the base LiteLLM's TypedDict uses, named so pep8-naming reads imageConfig as a key
class EditOptions(ImageEditOptionalRequestParams, typing_extensions.TypedDict, total=False):
    # LiteLLM keeps unmapped keys, and imageConfig loses its Any so no value here carries one
    imageConfig: dict[str, JsonValue] | None
    negative_prompt: str | None
    aspect_ratio: str | None
    seed: int | None
    mode: str | None
    strength: float | None
    style_preset: str | None
    left: int | None
    bottom: int | None
    right: int | None
    top: int | None
    creativity: float | None
    search_prompt: str | None
    grow_mask: int | None
    select_prompt: str | None
    control_strength: float | None
    composition_fidelity: float | None
    change_strength: float | None
    output_format: str | None


class FileSpec(TypedDict):
    content_base64: str


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    options: dict[str, JsonValue]
    drop_params: bool


class UrlCall(TypedDict):
    function: Literal["url"]
    model: str
    api_base: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    model: str
    api_key: NotRequired[str | None]
    headers: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class RequestCall(TypedDict):
    function: Literal["request"]
    model: str
    options: dict[str, JsonValue]
    drop_params: bool
    prompt: NotRequired[str | None]
    image: NotRequired[list[FileSpec] | None]
    files: NotRequired[dict[str, list[FileSpec]]]


class ResponseCall(TypedDict):
    function: Literal["response"]
    model: str
    status: int
    body: str
    created: int
    headers: NotRequired[dict[str, str]]


Call = MapCall | UrlCall | EnvironmentCall | RequestCall | ResponseCall


class Input(TypedDict):
    calls: list[Call]


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


OPTIONS: Final = TypeAdapter(EditOptions)
MAPPED: Final = TypeAdapter(dict[str, JsonValue])
HEADERS: Final = TypeAdapter(dict[str, str])
FORM: Final = TypeAdapter(tuple[dict[str, JsonValue], dict[str, bytes]])
HIDDEN: Final = TypeAdapter(Hidden)


class EditConfig(Protocol):
    @property
    def map_openai_params(self) -> Callable[[EditOptions, str, bool], dict[str, JsonValue]]: ...

    @property
    def get_complete_url(self) -> Callable[[str, str | None, dict[str, JsonValue]], str]: ...

    @property
    def validate_environment(self) -> Callable[
        [dict[str, str], str, str | None], dict[str, JsonValue]
    ]: ...

    @property
    def transform_image_edit_request(self) -> Callable[
        [
            str,
            str | None,
            bytes | None,
            dict[str, JsonValue | list[bytes]],
            GenericLiteLLMParams,
            dict[str, JsonValue],
        ],
        tuple[dict[str, JsonValue], object],  # ast-grep-ignore: no-loose-types
    ]: ...

    @property
    def transform_image_edit_response(self) -> Callable[
        [str, httpx.Response, Logging], ImageResponse
    ]: ...


class Sink(Logging):
    def __init__(self) -> None:
        pass


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def map_answer(call: MapCall) -> JsonValue:
    config: EditConfig = edit_config(call["model"])
    options = OPTIONS.validate_python(call["options"])
    try:
        mapped = MAPPED.validate_python(
            config.map_openai_params(options, call["model"], call["drop_params"])
        )
    except ValueError:
        return {"unsupported": True}
    return {"mapped": mapped}


def url_answer(call: UrlCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    config: EditConfig = edit_config(call["model"])
    params = call.get("litellm_params")
    url = config.get_complete_url(
        call["model"], call.get("api_base"), {} if params is None else params
    )
    return {"url": url}


def environment_answer(call: EnvironmentCall) -> JsonValue:
    environment = call.get("environment")
    apply_environment({} if environment is None else environment)
    config: EditConfig = edit_config(call["model"])
    headers: dict[str, str] = {}
    given = call.get("headers")
    if given is not None:
        headers.update(given)
    try:
        validated = HEADERS.validate_python(
            config.validate_environment(headers, call["model"], call.get("api_key"))
        )
    except ValueError:
        return {"missing_key": True}
    return {"headers": validated}


def first_image(specs: list[FileSpec] | None) -> bytes | None:
    if not specs:
        return None
    return base64.b64decode(specs[0]["content_base64"])


def request_answer(call: RequestCall) -> JsonValue:
    config: EditConfig = edit_config(call["model"])
    options = OPTIONS.validate_python(call["options"])
    try:
        mapped = MAPPED.validate_python(
            config.map_openai_params(options, call["model"], call["drop_params"])
        )
    except ValueError:
        return {"unsupported": True}
    form: dict[str, JsonValue | list[bytes]] = {}
    form.update(mapped)
    given = call.get("files")
    if given is not None:
        form.update(
            {
                name: [base64.b64decode(spec["content_base64"]) for spec in specs]
                for name, specs in given.items()
            }
        )
    data, files = FORM.validate_python(
        config.transform_image_edit_request(
            call["model"],
            call.get("prompt"),
            first_image(call.get("image")),
            form,
            GenericLiteLLMParams(),
            NO_HEADERS,
        )
    )
    encoded: list[JsonValue] = [
        {"name": name, "content_base64": base64.b64encode(content).decode("ascii")}
        for name, content in files.items()
    ]
    return {"data": data, "files": encoded}


def image_entries(response: ImageResponse) -> list[JsonValue]:
    images = response.data
    if images is None:
        return []
    return [
        {"b64_json": image.b64_json} for image in images if image.b64_json is not None
    ]


def response_answer(call: ResponseCall) -> JsonValue:
    config: EditConfig = edit_config(call["model"])
    time.time = partial(float, call["created"])
    upstream = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode()
    )
    response = config.transform_image_edit_response(call["model"], upstream, Sink())
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return {
        "response": {"created": response.created, "data": image_entries(response)},
        "cost": hidden["additional_headers"].get(COST_HEADER),
    }


def call_value(call: Call) -> JsonValue:
    match call["function"]:
        case "map":
            return map_answer(call)
        case "url":
            return url_answer(call)
        case "environment":
            return environment_answer(call)
        case "request":
            return request_answer(call)
        case "response":
            return response_answer(call)


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    return [call_value(call) for call in calls]

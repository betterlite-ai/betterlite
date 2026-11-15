import base64
import os
import time
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.stability.image_edit import get_stability_image_edit_config
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
ENVIRONMENT: Final[tuple[str, ...]] = ("STABILITY_API_KEY", "STABILITY_API_BASE")


class FileSpec(TypedDict):
    content_base64: str


class EditOptions(ImageEditOptionalRequestParams, total=False):
    # LiteLLM copies caller keys it does not transform, and pydantic would drop an undeclared one
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


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    options: NotRequired[dict[str, JsonValue]]
    drop_params: NotRequired[bool]


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
    prompt: NotRequired[str | None]
    image: NotRequired[list[FileSpec] | None]
    options: NotRequired[dict[str, JsonValue]]
    files: NotRequired[dict[str, list[FileSpec]]]
    drop_params: NotRequired[bool]


class ResponseCall(TypedDict):
    function: Literal["response"]
    model: str
    status: int
    headers: NotRequired[dict[str, str]]
    body: str
    created: int


Call = MapCall | UrlCall | EnvironmentCall | RequestCall | ResponseCall


class Input(TypedDict):
    calls: list[Call]


class Hidden(TypedDict, total=False):
    additional_headers: dict[str, JsonValue]


class EditConfig(Protocol):
    def map_openai_params(
        self,
        image_edit_optional_params: EditOptions,
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        model: str,
        api_base: str | None,
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None,
    ) -> dict[str, str]: ...

    def transform_image_edit_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        prompt: str | None,
        image: bytes | None,
        image_edit_optional_request_params: dict[str, object],
        litellm_params: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> tuple[object, object]: ...

    def transform_image_edit_response(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: Logging,
    ) -> ImageResponse: ...


OPTIONS: Final = TypeAdapter(EditOptions)
DATA: Final = TypeAdapter(dict[str, JsonValue])
FILES: Final = TypeAdapter(dict[str, bytes])
JSON: Final = TypeAdapter(JsonValue)
HIDDEN: Final = TypeAdapter(Hidden)


class Logger(Logging):
    @override
    def __init__(self) -> None:
        pass

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args, params


def load_config(model: str) -> EditConfig:
    return get_stability_image_edit_config(model)


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT:
        os.environ.pop(name, None)
        value = environment.get(name)
        if value is not None:
            os.environ[name] = value


def map_answer(call: MapCall) -> JsonValue:
    config = load_config(call["model"])
    options = OPTIONS.validate_python(call.get("options", {}))
    try:
        mapped = config.map_openai_params(options, call["model"], call.get("drop_params", False))
    except ValueError:
        return {"unsupported": True}
    return {"mapped": mapped}


def url_answer(call: UrlCall) -> JsonValue:
    apply_environment(call.get("environment", {}))
    config = load_config(call["model"])
    url = config.get_complete_url(
        call["model"], call.get("api_base"), call.get("litellm_params", {})
    )
    return {"url": url}


def environment_answer(call: EnvironmentCall) -> JsonValue:
    apply_environment(call.get("environment", {}))
    config = load_config(call["model"])
    headers: dict[str, str] = dict(call.get("headers", {}))
    try:
        validated = config.validate_environment(headers, call["model"], call.get("api_key"))
    except ValueError:
        return {"missing_key": True}
    return {"headers": validated}


def file_bytes(spec: FileSpec) -> bytes:
    return base64.b64decode(spec["content_base64"])


def first_image(specs: list[FileSpec] | None) -> bytes | None:
    if not specs:
        return None
    return file_bytes(specs[0])


def files_answer(files: object) -> JsonValue:
    values = FILES.validate_python(files)
    return JSON.validate_python(
        [
            {"name": name, "content_base64": base64.b64encode(content).decode("ascii")}
            for name, content in values.items()
        ]
    )


def request_answer(call: RequestCall) -> JsonValue:
    config = load_config(call["model"])
    options = OPTIONS.validate_python(call.get("options", {}))
    try:
        mapped = config.map_openai_params(options, call["model"], call.get("drop_params", False))
    except ValueError:
        return {"unsupported": True}
    form: dict[str, object] = {}
    for name, value in mapped.items():
        form[name] = value
    for name, specs in call.get("files", {}).items():
        form[name] = [file_bytes(spec) for spec in specs]
    data, files = config.transform_image_edit_request(  # ast-grep-ignore: no-long-names
        call["model"],
        call.get("prompt"),
        first_image(call.get("image")),
        form,
        GenericLiteLLMParams(),
        {},
    )
    return {"data": DATA.validate_python(data), "files": files_answer(files)}


def response_value(response: ImageResponse) -> JsonValue:
    data = response.data
    if data is None:
        return {"created": response.created, "data": []}
    images: list[JsonValue] = []
    for image in data:
        entry: dict[str, JsonValue] = {}
        if image.b64_json is not None:
            entry["b64_json"] = image.b64_json
        images.append(entry)
    return {"created": response.created, "data": images}


def response_answer(call: ResponseCall) -> JsonValue:
    config = load_config(call["model"])
    time.time = partial(float, call["created"])
    upstream = httpx.Response(
        call["status"],
        headers=call.get("headers", {}),
        content=call["body"].encode(),
    )
    response = config.transform_image_edit_response(  # ast-grep-ignore: no-long-names
        call["model"], upstream, Logger()
    )
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    additional = hidden.get("additional_headers", {})
    return {"response": response_value(response), "cost": additional.get(COST_HEADER)}


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
    return [call_value(call) for call in input_value["calls"]]

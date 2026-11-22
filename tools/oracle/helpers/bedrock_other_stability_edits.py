from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
import typing_extensions
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_edit.amazon_nova_canvas_image_edit_transformation import (
    BedrockAmazonNovaCanvasImageEditConfig as NovaConfig,
)
from litellm.llms.bedrock.image_edit.amazon_nova_canvas_image_edit_transformation import (
    get_bedrock_image_edit_config_for_model as edit_config_for,
)
from litellm.llms.bedrock.image_edit.stability_transformation import (
    BedrockStabilityImageEditConfig as StabilityConfig,
)
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.images.main import ImageEditOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
MODEL: Final = "stability.stable-image-inpaint-v1:0"
NO_HEADERS: Final[dict[str, JsonValue]] = {}


# LiteLLM leaves the stability option keys untyped, so their values stay JsonValue
class EditOptions(ImageEditOptionalRequestParams, typing_extensions.TypedDict, total=False):
    imageConfig: dict[str, JsonValue] | None
    negative_prompt: JsonValue
    aspect_ratio: JsonValue
    output_format: JsonValue
    model: JsonValue
    mode: JsonValue
    style_preset: JsonValue
    select_prompt: JsonValue
    search_prompt: JsonValue
    init_image: JsonValue
    style_image: JsonValue
    left: JsonValue
    right: JsonValue
    up: JsonValue
    down: JsonValue
    seed: JsonValue
    strength: JsonValue
    creativity: JsonValue
    control_strength: JsonValue
    grow_mask: JsonValue
    fidelity: JsonValue
    composition_fidelity: JsonValue
    style_strength: JsonValue
    change_strength: JsonValue


class ImageSpec(TypedDict):
    value: str


class ModelSelectionCall(TypedDict):
    function: Literal["model_selection"]
    models: list[str]


class FlagCall(TypedDict):
    function: Literal["supported_params", "multipart", "url"]


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    options: dict[str, JsonValue]
    drop_params: bool


class RequestCall(TypedDict):
    function: Literal["request"]
    model: str
    options: dict[str, JsonValue]
    drop_params: bool
    prompt: NotRequired[str | None]
    image: NotRequired[ImageSpec | None]


class ResponseCall(TypedDict):
    function: Literal["response"]
    model: str
    status: int
    body: str


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    headers: dict[str, str] | None


class ErrorClassCall(TypedDict):
    function: Literal["error_class"]
    message: str
    status: int
    headers: dict[str, str]


ConfigurationCall = ModelSelectionCall | FlagCall | EnvironmentCall
ExchangeCall = MapCall | RequestCall | ResponseCall | ErrorClassCall
Call = ConfigurationCall | ExchangeCall


class Input(TypedDict):
    calls: list[Call]


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


OPTIONS: Final = TypeAdapter(EditOptions)
MAPPED: Final = TypeAdapter(dict[str, JsonValue])
SUPPORTED: Final = TypeAdapter(list[JsonValue])
FORM: Final = TypeAdapter(tuple[dict[str, JsonValue], dict[str, JsonValue]])
HIDDEN: Final = TypeAdapter(Hidden)


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: dict[str, str] | httpx.Headers | None


class EditConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Callable[[str], list[str]]: ...

    @property
    def get_error_class(self) -> Callable[[str, int, dict[str, str]], ErrorView]: ...

    @property
    def map_openai_params(
        self,
    ) -> Callable[[EditOptions, str, bool], dict[str, JsonValue]]: ...

    @property
    def transform_image_edit_request(self) -> Callable[
        [
            str,
            str | None,
            bytes | None,
            dict[str, JsonValue],
            GenericLiteLLMParams,
            dict[str, JsonValue],
        ],
        tuple[dict[str, JsonValue], object],  # ast-grep-ignore: no-loose-types
    ]: ...

    @property
    def transform_image_edit_response(
        self,
    ) -> Callable[[str, httpx.Response, Logging], ImageResponse]: ...

    @property
    def use_multipart_form_data(self) -> Callable[[], bool]: ...

    @property
    def get_complete_url(
        self,
    ) -> Callable[[str, str | None, dict[str, JsonValue]], str]: ...

    @property
    def validate_environment(
        self,
    ) -> Callable[[dict[str, str], str, str | None], dict[str, JsonValue]]: ...


class Sink(Logging):
    def __init__(self) -> None:
        pass


def stability_edit_model(model: str) -> bool:
    try:
        config = edit_config_for(model)
    except ValueError:
        return False
    match config:
        case StabilityConfig():
            return True
        case NovaConfig() | BaseImageEditConfig():
            return False


def image_entries(response: ImageResponse) -> list[JsonValue]:
    images = response.data
    if images is None:
        return []
    return [
        {"b64_json": image.b64_json} for image in images if image.b64_json is not None
    ]


def map_answer(config: EditConfig, call: MapCall) -> JsonValue:
    options = OPTIONS.validate_python(call["options"])
    try:
        mapped = MAPPED.validate_python(
            config.map_openai_params(options, call["model"], call["drop_params"])
        )
    except ValueError:
        return {"unsupported": True}
    return {"mapped": mapped}


def request_answer(config: EditConfig, call: RequestCall) -> JsonValue:
    options = OPTIONS.validate_python(call["options"])
    try:
        mapped = MAPPED.validate_python(
            config.map_openai_params(options, call["model"], call["drop_params"])
        )
    except ValueError:
        return {"unsupported": True}
    spec = call.get("image")
    image = None if spec is None else spec["value"].encode()
    body, files = FORM.validate_python(
        config.transform_image_edit_request(
            call["model"],
            call.get("prompt"),
            image,
            mapped,
            GenericLiteLLMParams(),
            NO_HEADERS,
        )
    )
    return {"body": body, "files": len(files)}


def response_answer(config: EditConfig, call: ResponseCall) -> JsonValue:
    upstream = httpx.Response(call["status"], headers=None, content=call["body"].encode())
    try:
        response = config.transform_image_edit_response(call["model"], upstream, Sink())
    except BedrockError as error:
        return {"ok": False, "status": error.status_code}
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return {
        "ok": True,
        "images": image_entries(response),
        "cost": hidden["additional_headers"].get(COST_HEADER),
    }


def environment_answer(config: EditConfig, call: EnvironmentCall) -> JsonValue:
    given = call["headers"]
    headers: dict[str, str] = {} if given is None else given
    return {"headers": config.validate_environment(headers, MODEL, None)}


def error_class_answer(config: EditConfig, call: ErrorClassCall) -> JsonValue:
    carried = config.get_error_class(call["message"], call["status"], call["headers"])
    return {
        "status": carried.status_code,
        "message": carried.message,
        "headers_present": carried.headers is not None,
    }


def call_value(call: Call) -> JsonValue:
    config: EditConfig = edit_config_for(MODEL)
    match call["function"]:
        case "model_selection":
            answer: JsonValue = [
                stability_edit_model(model) for model in call["models"]
            ]
        case "supported_params":
            supported = SUPPORTED.validate_python(
                config.get_supported_openai_params(MODEL)
            )
            answer = {"supported": supported}
        case "multipart":
            answer = {"multipart": config.use_multipart_form_data()}
        case "url":
            answer = {"url": config.get_complete_url(MODEL, None, {})}
        case "environment":
            answer = environment_answer(config, call)
        case "map":
            answer = map_answer(config, call)
        case "request":
            answer = request_answer(config, call)
        case "response":
            answer = response_answer(config, call)
        case "error_class":
            answer = error_class_answer(config, call)
    return answer


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    return [call_value(call) for call in calls]

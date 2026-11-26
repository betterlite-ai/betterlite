import logging
from collections.abc import Callable, Generator, Sized
from contextlib import contextmanager
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.bedrock.image_edit.amazon_nova_canvas_image_edit_transformation import (
    BedrockAmazonNovaCanvasImageEditConfig as NovaConfig,
    get_bedrock_image_edit_config_for_model as edit_config_for,
)
from litellm.llms.bedrock.image_edit.stability_transformation import (
    BedrockStabilityImageEditConfig as StabilityConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

MODEL: Final = "amazon.nova-canvas-v1:0"
NO_HEADERS: Final[dict[str, str]] = {}

Media: TypeAlias = bytes | str | tuple[int, int]
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MAPPED: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
HEADERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
SUPPORTED: TypeAdapter[list[str]] = TypeAdapter(list[str])
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)


class BytesSpec(TypedDict):
    kind: Literal["bytes"]
    data: str


class TextSpec(TypedDict):
    kind: Literal["text"]
    text: str


class TupleSpec(TypedDict):
    kind: Literal["tuple"]


FileSpec: TypeAlias = BytesSpec | TextSpec | TupleSpec


class FlagCall(TypedDict):
    function: Literal["supported_params", "multipart", "url"]


class MapCall(TypedDict):
    function: Literal["map"]
    options: dict[str, JsonValue]
    drop_params: bool


class TransformCall(TypedDict):
    function: Literal["transform"]
    prompt: str | None
    image: FileSpec | None
    mask: FileSpec | None
    options: dict[str, JsonValue]
    drop_params: bool


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    headers: dict[str, str] | None


class ErrorClassCall(TypedDict):
    function: Literal["error_class"]
    message: str
    status: int
    headers: dict[str, str]


class ModelsCall(TypedDict):
    function: Literal["eligibility", "selection"]
    models: list[str]


Call: TypeAlias = FlagCall | MapCall | TransformCall | EnvironmentCall | ErrorClassCall | ModelsCall


class Input(TypedDict):
    model_cost: dict[str, JsonValue]
    calls: list[Call]


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: dict[str, str] | httpx.Headers | None


# The object is built by pydantic so the protocol carries the signatures the helper calls.
@runtime_checkable
class EditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[[dict[str, JsonValue], str, bool], dict[str, JsonValue]]
    transform_image_edit_request: Callable[
        [
            str,
            str | None,
            Media | None,
            dict[str, JsonValue | Media],
            GenericLiteLLMParams,
            dict[str, str],
        ],
        tuple[dict[str, JsonValue], Sized],
    ]
    use_multipart_form_data: Callable[[], bool]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    validate_environment: Callable[[dict[str, str], str, str | None], dict[str, str]]
    get_error_class: Callable[[str, int, httpx.Headers], ErrorView]


CONFIG: EditConfig = TypeAdapter[EditConfig](EditConfig, config=ARBITRARY).validate_python(
    NovaConfig()
)


class DebugMessages(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.DEBUG:
            self.messages.append(record.getMessage())


# The logger's own handlers print to stdout, which carries the helper answer, so they step aside.
@contextmanager
def capturing() -> Generator[DebugMessages]:
    messages = DebugMessages()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    previous_handlers = logger.handlers[:]
    logger.handlers = [messages]
    logger.setLevel(logging.DEBUG)
    try:
        yield messages
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)


def file_value(spec: FileSpec) -> Media:
    match spec["kind"]:
        case "bytes":
            return spec["data"].encode()
        case "text":
            return spec["text"]
        case "tuple":
            return (0, 0)


def url_answer() -> JsonValue:
    try:
        url = CONFIG.get_complete_url(MODEL, None, {})
    except NotImplementedError as error:
        return JSON.validate_python({"error": str(error)})
    return JSON.validate_python({"url": url})


def map_answer(call: MapCall) -> JsonValue:
    with capturing() as captured:
        mapped = MAPPED.validate_python(
            CONFIG.map_openai_params(call["options"], MODEL, call["drop_params"])
        )
    message = captured.messages[-1] if captured.messages else None
    return JSON.validate_python({"mapped": mapped, "message": message})


def transform_answer(call: TransformCall) -> JsonValue:
    image_spec = call["image"]
    image = None if image_spec is None else file_value(image_spec)
    mapped = CONFIG.map_openai_params(call["options"], MODEL, call["drop_params"])
    optional: dict[str, JsonValue | Media] = dict(mapped)
    mask_spec = call["mask"]
    if mask_spec is not None:
        optional["mask"] = file_value(mask_spec)
    try:
        body, files = CONFIG.transform_image_edit_request(
            MODEL, call["prompt"], image, optional, GenericLiteLLMParams(), NO_HEADERS
        )
    except ValueError as error:
        return JSON.validate_python({"error": str(error)})
    return JSON.validate_python({"body": MAPPED.validate_python(body), "files": len(files)})


def environment_answer(call: EnvironmentCall) -> JsonValue:
    given_spec = call["headers"]
    given: dict[str, str] = {} if given_spec is None else given_spec
    return JSON.validate_python({"headers": CONFIG.validate_environment(given, MODEL, None)})


def error_answer(call: ErrorClassCall) -> JsonValue:
    carried = CONFIG.get_error_class(
        call["message"], call["status"], httpx.Headers(call["headers"])
    )
    carried_headers = carried.headers
    headers = (
        HEADERS.validate_python(dict(carried_headers))
        if isinstance(carried_headers, httpx.Headers)
        else None
    )
    answer: dict[str, JsonValue] = {
        "status": carried.status_code,
        "message": carried.message,
        "headers": headers,
    }
    return JSON.validate_python(answer)


def eligible(model: str) -> bool:
    try:
        config = edit_config_for(model)
    except ValueError:
        return False
    return isinstance(config, NovaConfig)


def eligibility_answer(call: ModelsCall) -> JsonValue:
    return JSON.validate_python({"eligible": [eligible(model) for model in call["models"]]})


def selected(model: str) -> JsonValue:
    try:
        config = edit_config_for(model)
    except ValueError as error:
        return {"error": str(error)}
    if isinstance(config, StabilityConfig):
        return {"config": "stability"}
    if isinstance(config, NovaConfig):
        return {"config": "nova_canvas"}
    return {"config": "other"}


def selection_answer(call: ModelsCall) -> JsonValue:
    return JSON.validate_python({"selected": [selected(model) for model in call["models"]]})


def flag_answer(call: FlagCall) -> JsonValue:
    match call["function"]:
        case "supported_params":
            supported = SUPPORTED.validate_python(CONFIG.get_supported_openai_params(MODEL))
            return JSON.validate_python({"supported": supported})
        case "multipart":
            return JSON.validate_python({"multipart": CONFIG.use_multipart_form_data()})
        case "url":
            return url_answer()


def payload_answer(call: MapCall | TransformCall) -> JsonValue:
    match call["function"]:
        case "map":
            return map_answer(call)
        case "transform":
            return transform_answer(call)


def contract_answer(call: EnvironmentCall | ErrorClassCall) -> JsonValue:
    match call["function"]:
        case "environment":
            return environment_answer(call)
        case "error_class":
            return error_answer(call)


def models_answer(call: ModelsCall) -> JsonValue:
    match call["function"]:
        case "eligibility":
            return eligibility_answer(call)
        case "selection":
            return selection_answer(call)


def call_value(call: Call) -> JsonValue:
    answer: JsonValue = None
    match call["function"]:
        case "supported_params" | "multipart" | "url":
            answer = flag_answer(call)
        case "map" | "transform":
            answer = payload_answer(call)
        case "environment" | "error_class":
            answer = contract_answer(call)
        case "eligibility" | "selection":
            answer = models_answer(call)
    return answer


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    return JSON.validate_python([call_value(call) for call in input_value["calls"]])

import os
from collections.abc import Callable, Sequence
from typing import Final, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.minimax.text_to_speech.transformation import (
    MinimaxTextToSpeechConfig as Speech,
)
from litellm.llms.modelscope.image_generation.transformation import (
    ModelScopeImageGenerationConfig as Images,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

Object = dict[str, JsonValue]


class Step(TypedDict, total=False):
    kind: str
    model: str
    input: str
    voice: str | dict[str, JsonValue]
    params: Object
    api_base: str
    env_base: str
    api_key: str
    global_key: str
    env_key: str
    status: int
    headers: dict[str, str]
    body: str
    drop_params: bool
    prompt: str
    created: int


class Input(TypedDict):
    steps: list[Step]


class SpeechParamsStep(TypedDict, total=False):
    voice: str | dict[str, JsonValue]
    params: Object


class SpeechBodyStep(TypedDict):
    model: str
    input: str
    voice: NotRequired[str | dict[str, JsonValue]]
    params: NotRequired[Object]


class UrlStep(TypedDict, total=False):
    api_base: str
    env_base: str


class SpeechHeadersStep(TypedDict, total=False):
    api_key: str
    global_key: str
    env_key: str


class ImageHeadersStep(TypedDict, total=False):
    api_key: str
    env_key: str


class SpeechReplyStep(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]


class ImageReplyStep(TypedDict):
    status: int
    body: str
    created: NotRequired[int]


class ImageParamsStep(TypedDict):
    model: str
    params: NotRequired[Object]
    drop_params: NotRequired[bool]


class ImageBodyStep(TypedDict):
    model: str
    prompt: str
    params: NotRequired[Object]


class SpeechRequestData(TypedDict, total=False):
    dict_body: Object


class BinaryReply(Protocol):
    response: httpx.Response

    @property
    def content(self) -> bytes: ...


class ImageDatum(Protocol):
    url: str | None
    b64_json: str | None
    revised_prompt: str | None


class ImageReply(Protocol):
    created: int

    @property
    def data(self) -> Sequence[ImageDatum]: ...


class SpeechAdapter:
    map_params: Callable[
        [str, Object, str | dict[str, JsonValue] | None, bool, Object | None],
        tuple[str | None, Object],
    ]
    request_body: Callable[
        [str, str, str | None, Object, Object, dict[str, str]], SpeechRequestData
    ]
    environment: Callable[[dict[str, str], str, str | None], dict[str, str]]
    url: Callable[[str, str | None, Object], str]
    response: Callable[[str, httpx.Response, LiteLLMLoggingObj], BinaryReply]

    def __init__(self, config: Speech) -> None:
        self.map_params = config.map_openai_params
        self.request_body = config.transform_text_to_speech_request
        self.environment = config.validate_environment
        self.url = config.get_complete_url
        self.response = config.transform_text_to_speech_response


class ImageAdapter:
    supported_params: Callable[[str], Sequence[str]]
    map_params: Callable[[Object, Object, str, bool], Object]
    url: Callable[[str | None, str | None, str, Object, Object], str]
    environment: Callable[
        [dict[str, str], str, list[AllMessageValues], Object, Object, str | None],
        dict[str, str],
    ]
    request_body: Callable[[str, str, Object, Object, dict[str, str]], Object]
    response: Callable[
        [
            str,
            httpx.Response,
            ImageResponse,
            LiteLLMLoggingObj,
            Object,
            Object,
            Object,
            JsonValue,
        ],
        ImageReply,
    ]

    def __init__(self, config: Images) -> None:
        self.supported_params = config.get_supported_openai_params
        self.map_params = config.map_openai_params
        self.url = config.get_complete_url
        self.environment = config.validate_environment
        self.request_body = config.transform_image_generation_request
        self.response = config.transform_image_generation_response


class SpeechFactory(Protocol):
    def __call__(self) -> Speech: ...


class ImageFactory(Protocol):
    def __call__(self) -> Images: ...


class ImageResponseFactory(Protocol):
    def __call__(self, *, created: int | None = None) -> ImageResponse: ...


class Recorder(LiteLLMLoggingObj):
    def __init__(self) -> None:
        self.model_call_details: Object = {}


class UnsupportedOptionError(Exception):
    status_code: int

    def __init__(self, name: str, model: str) -> None:
        super().__init__(f"parameter {name} is not supported for model {model}")
        self.status_code = 400


SPEECH: Final[SpeechFactory] = Speech
IMAGES: Final[ImageFactory] = Images
IMAGE_RESPONSE: Final[ImageResponseFactory] = ImageResponse
TEXT_HEADERS = TypeAdapter(dict[str, str])
NO_DROP: Final[bool] = False


def install_environment(values: dict[str, str | None]) -> None:
    os.environ.clear()
    for name, value in values.items():
        if value is not None:
            os.environ[name] = value


def object_or_empty(value: Object | None) -> Object:
    if value is None:
        return {}
    return value


def speech_params(step: Step) -> JsonValue:
    fields = TypeAdapter(SpeechParamsStep).validate_python(step)
    mapped_voice, mapped = SpeechAdapter(SPEECH()).map_params(
        "",
        object_or_empty(fields.get("params")),
        fields.get("voice"),
        NO_DROP,
        None,
    )
    return {"voice": mapped_voice, "params": mapped}


def speech_body(step: Step) -> JsonValue:
    fields = TypeAdapter(SpeechBodyStep).validate_python(step)
    speech = SpeechAdapter(SPEECH())
    mapped_voice, mapped = speech.map_params(
        fields["model"],
        object_or_empty(fields.get("params")),
        fields.get("voice"),
        NO_DROP,
        None,
    )
    request = speech.request_body(
        fields["model"],
        fields["input"],
        mapped_voice,
        mapped,
        {},
        {},
    )
    return request["dict_body"]


def speech_url(step: Step) -> JsonValue:
    fields = TypeAdapter(UrlStep).validate_python(step)
    install_environment({"MINIMAX_API_BASE": fields.get("env_base")})
    return SpeechAdapter(SPEECH()).url("", fields.get("api_base"), {})


def speech_headers(step: Step) -> JsonValue:
    fields = TypeAdapter(SpeechHeadersStep).validate_python(step)
    install_environment({"MINIMAX_API_KEY": fields.get("env_key")})
    litellm.api_key = fields.get("global_key")
    headers = SpeechAdapter(SPEECH()).environment({}, "", fields.get("api_key"))
    return TEXT_HEADERS.validate_python(headers)


def speech_reply(step: Step) -> JsonValue:
    fields = TypeAdapter(SpeechReplyStep).validate_python(step)
    response = httpx.Response(
        fields["status"],
        content=fields["body"].encode(),
        headers=fields.get("headers"),
        request=httpx.Request("POST", "https://host"),
    )
    result = SpeechAdapter(SPEECH()).response("", response, Recorder())
    return {
        "status": result.response.status_code,
        "content_type": result.response.headers.get("content-type"),
        "headers": TEXT_HEADERS.validate_python(result.response.headers),
        "body": result.content.hex(),
    }


def image_params(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageParamsStep).validate_python(step)
    images = ImageAdapter(IMAGES())
    params = object_or_empty(fields.get("params"))
    supported = images.supported_params(fields["model"])
    drop_params = fields.get("drop_params")
    if drop_params is None:
        drop_params = False
    kept: Object = {}
    for name, value in params.items():
        if name in supported:
            kept[name] = value
        elif not drop_params:
            raise UnsupportedOptionError(name, fields["model"])
    optional: Object = {}
    return {"params": images.map_params(kept, optional, fields["model"], drop_params)}


def image_url(step: Step) -> JsonValue:
    fields = TypeAdapter(UrlStep).validate_python(step)
    install_environment({"MODELSCOPE_API_BASE": fields.get("env_base")})
    return ImageAdapter(IMAGES()).url(fields.get("api_base"), None, "", {}, {})


def image_headers(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageHeadersStep).validate_python(step)
    install_environment({"MODELSCOPE_API_KEY": fields.get("env_key")})
    headers = ImageAdapter(IMAGES()).environment(
        {}, "", [], {}, {}, fields.get("api_key")
    )
    return TEXT_HEADERS.validate_python(headers)


def image_body(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageBodyStep).validate_python(step)
    return ImageAdapter(IMAGES()).request_body(
        fields["model"],
        fields["prompt"],
        object_or_empty(fields.get("params")),
        {},
        {},
    )


def image_datum(image: ImageDatum) -> Object:
    datum: Object = {}
    if image.url is not None:
        datum["url"] = image.url
    if image.b64_json is not None:
        datum["b64_json"] = image.b64_json
    if image.revised_prompt is not None:
        datum["revised_prompt"] = image.revised_prompt
    return datum


def image_reply(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageReplyStep).validate_python(step)
    response = httpx.Response(
        fields["status"],
        content=fields["body"].encode(),
        request=httpx.Request("POST", "https://host"),
    )
    reply = ImageAdapter(IMAGES()).response(
        "",
        response,
        IMAGE_RESPONSE(created=fields.get("created")),
        Recorder(),
        {},
        {},
        {},
        None,
    )
    return {"created": reply.created, "data": [image_datum(image) for image in reply.data]}


HANDLERS: dict[str, Callable[[Step], JsonValue]] = {
    "speech_params": speech_params,
    "speech_body": speech_body,
    "speech_url": speech_url,
    "speech_headers": speech_headers,
    "speech_reply": speech_reply,
    "image_params": image_params,
    "image_url": image_url,
    "image_headers": image_headers,
    "image_body": image_body,
    "image_reply": image_reply,
}


def run(value: dict[str, JsonValue]) -> JsonValue:
    request = TypeAdapter(Input).validate_python(value)
    return [HANDLERS[step["kind"]](step) for step in request["steps"]]

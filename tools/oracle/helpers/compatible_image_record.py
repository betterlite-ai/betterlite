import asyncio
import json
import os
from collections.abc import Awaitable, Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, TypeVar

import httpx
import litellm
from litellm.exceptions import ModelNotMappedError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.aiml.image_generation.cost_calculator import cost_calculator
from litellm.llms.aiml.image_generation.transformation import AimlImageGenerationConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_generation.transformation import BaseImageGenerationConfig
from litellm.llms.cometapi.image_generation.transformation import CometAPIImageGenerationConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import OpenAIImageGenerationOptionalParams
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.image import Image as OpenAIImage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EMPTY: Final[dict[str, JsonValue]] = {}
GENERATED: Final = "https://example.invalid/generated.png"
NO_FAKE_STREAM: Final = False
T = TypeVar("T")
Object = dict[str, JsonValue]
HostName = Literal["aiml", "cometapi"]
Config = AimlImageGenerationConfig | CometAPIImageGenerationConfig


class ReplyFixture(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class OptionsCall(TypedDict):
    host: HostName
    model: str
    non_default_params: Object
    optional_params: Object
    drop_params: bool


class RequestCall(TypedDict):
    host: HostName
    model: str
    prompt: str
    optional_params: Object
    api_key: str | None
    api_base: str | None
    env: dict[str, str]
    reply: ReplyFixture


class PriceCall(TypedDict):
    host: HostName
    model: str
    images: int


class Input(TypedDict):
    function: Literal["options", "request", "price"]
    options: NotRequired[list[OptionsCall]]
    requests: NotRequired[list[RequestCall]]
    prices: NotRequired[list[PriceCall]]


class OptionMapper(Protocol):
    @property
    def get_supported_openai_params(
        self,
    ) -> Callable[[str], list[OpenAIImageGenerationOptionalParams]]: ...

    @property
    def map_openai_params(self) -> Callable[[Object, Object, str, bool], Object]: ...


class ImageHandler(Protocol):
    @property
    def async_image_generation_handler(
        self,
    ) -> Callable[
        [
            str,
            str,
            BaseImageGenerationConfig,
            Object,
            str,
            Object,
            Logging,
            float,
            dict[str, object] | None,
            dict[str, object] | None,
            AsyncHTTPHandler,
            bool,
            dict[str, object] | None,
            str | None,
        ],
        Awaitable[ImageResponse],
    ]: ...


class PriceCalculator(Protocol):
    def __call__(self, model: str, image_response: ImageResponse) -> float: ...


CONFIGS: Final[dict[HostName, Config]] = {
    "aiml": AimlImageGenerationConfig(),
    "cometapi": CometAPIImageGenerationConfig(),
}
PLAIN: Final[dict[HostName, OptionMapper]] = {
    "aiml": AimlImageGenerationConfig(),
    "cometapi": CometAPIImageGenerationConfig(),
}
HANDLER: ImageHandler = BaseLLMHTTPHandler()
PRICE: Final[PriceCalculator] = cost_calculator


class ImageLogging(Logging):
    def __init__(self) -> None:
        ...

    @override
    def pre_call(
        self,
        input: str,
        api_key: str,
        model: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        ...


class RecordingTransport:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.reply_body = body
        self.url = ""
        self.authorization = ""
        self.content_type = ""
        self.body: JsonValue = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.authorization = request.headers["authorization"]
        self.content_type = request.headers["content-type"]
        self.body = JSON.validate_python(json.loads(request.content))
        return httpx.Response(self.status, text=self.reply_body, request=request)


def required(value: T | None, name: str) -> T:
    if value is None:
        raise ValueError(f"{name} is required")
    return value


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value


def failed(status: int | None) -> JsonValue:
    return {"ok": False, "status": status, "value": None}


def succeeded(value: JsonValue) -> JsonValue:
    return {"ok": True, "status": None, "value": value}


def image_spec(image: OpenAIImage) -> JsonValue:
    return JSON.validate_python(
        {"url": image.url, "b64_json": image.b64_json, "revised_prompt": image.revised_prompt}
    )


def data_view(response: ImageResponse) -> JsonValue:
    data: list[OpenAIImage] = list(response.data) if response.data is not None else []
    return {"data": [image_spec(image) for image in data]}


def options_value(call: OptionsCall) -> JsonValue:
    mapper = PLAIN[call["host"]]
    supported = mapper.get_supported_openai_params(call["model"])
    try:
        mapped = mapper.map_openai_params(
            dict(call["non_default_params"]),
            dict(call["optional_params"]),
            call["model"],
            call["drop_params"],
        )
    except ValueError:
        return failed(None)
    return succeeded(
        {
            "supported": JSON.validate_python(supported),
            "mapped": JSON.validate_python(mapped),
        }
    )


async def request_call(call: RequestCall) -> JsonValue:
    apply_env(call["env"])
    transport = RecordingTransport(call["reply"]["status"], call["reply"]["body"])
    client = AsyncHTTPHandler(transport=httpx.MockTransport(transport.handle))
    litellm_params: Object = {"api_base": call["api_base"]}
    try:
        response = await HANDLER.async_image_generation_handler(
            call["model"],
            call["prompt"],
            CONFIGS[call["host"]],
            dict(call["optional_params"]),
            call["host"],
            litellm_params,
            ImageLogging(),
            30.0,
            None,
            None,
            client,
            NO_FAKE_STREAM,
            None,
            call["api_key"],
        )
    except BaseLLMException as error:
        return failed(error.status_code)
    except (ValueError, TypeError, AttributeError):
        return failed(None)
    return succeeded(
        {
            "url": transport.url,
            "headers": {
                "authorization": transport.authorization,
                "content-type": transport.content_type,
            },
            "body": transport.body,
            "reply": data_view(response),
        }
    )


def price_value(call: PriceCall) -> JsonValue:
    images = [ImageObject(url=GENERATED) for _ in range(call["images"])]
    try:
        cost = PRICE(call["model"], ImageResponse(data=images))
    except (ModelNotMappedError, ValueError, TypeError):
        return failed(None)
    return succeeded({"cost": str(cost)})


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "options":
        return [
            options_value(call)
            for call in required(input_value.get("options"), "options")
        ]
    if function == "request":
        requests = required(input_value.get("requests"), "requests")
        return [asyncio.run(request_call(call)) for call in requests]
    return [price_value(call) for call in required(input_value.get("prices"), "prices")]

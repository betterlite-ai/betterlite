import asyncio
import os
from collections.abc import Awaitable, Callable, Sequence
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.aiml.image_generation.cost_calculator import cost_calculator
from litellm.llms.aiml.image_generation.transformation import AimlImageGenerationConfig
from litellm.llms.base_llm.image_generation.transformation import BaseImageGenerationConfig
from litellm.llms.cometapi.image_generation.transformation import CometAPIImageGenerationConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.image import Image as OpenAIImage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
GENERATED: Final = "https://example.invalid/generated.png"
NO_FAKE_STREAM: Final = False
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
    prices: dict[str, Object]


class Input(TypedDict):
    function: Literal["options", "request", "price"]
    options: list[OptionsCall]
    requests: list[RequestCall]
    prices: list[PriceCall]


class OptionMapper(Protocol):
    @property
    def get_supported_openai_params(self) -> Callable[[str], Sequence[str]]: ...

    @property
    def map_openai_params(self) -> Callable[[Object, Object, str, bool], Object]: ...


class ImageHandler(Protocol):
    @property
    def async_image_generation_handler(
        self,
    ) -> Callable[
        [
            str, str, BaseImageGenerationConfig, Object, str, Object, Logging, float,
            None, None, AsyncHTTPHandler, bool, None, str | None,
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
        additional_args: Object | None = None,
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
        self.body = JSON.validate_json(request.content)
        return httpx.Response(self.status, text=self.reply_body, request=request)


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value


def image_spec(image: OpenAIImage) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "url": image.url,
        "b64_json": image.b64_json,
        "revised_prompt": image.revised_prompt,
    }
    return JSON.validate_python(fields)


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
        return {"ok": False, "status": None, "value": None}
    value: JsonValue = {
        "supported": JSON.validate_python(supported),
        "mapped": JSON.validate_python(mapped),
    }
    return {"ok": True, "status": None, "value": value}


async def request_call(call: RequestCall) -> JsonValue:
    apply_env(call["env"])
    transport = RecordingTransport(call["reply"]["status"], call["reply"]["body"])
    client = AsyncHTTPHandler(transport=httpx.MockTransport(transport.handle))
    litellm_params: Object = {"api_base": call["api_base"]}
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
    value: JsonValue = {
        "url": transport.url,
        "headers": {
            "authorization": transport.authorization,
            "content-type": transport.content_type,
        },
        "body": transport.body,
        "reply": data_view(response),
    }
    return {"ok": True, "status": None, "value": value}


def price_value(call: PriceCall) -> JsonValue:
    images = [ImageObject(url=GENERATED) for _ in range(call["images"])]
    cost = PRICE(call["model"], ImageResponse(data=images))
    value: JsonValue = {"cost": str(cost)}
    return {"ok": True, "status": None, "value": value}


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "options":
        return [options_value(call) for call in input_value["options"]]
    if function == "request":
        requests = input_value["requests"]
        return [asyncio.run(request_call(call)) for call in requests]
    return [price_value(call) for call in input_value["prices"]]

import os
from collections.abc import Callable
from dataclasses import dataclass
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.litellm_proxy.chat.transformation import LiteLLMProxyChatConfig
from litellm.llms.litellm_proxy.image_edit.transformation import LiteLLMProxyImageEditConfig
from litellm.llms.litellm_proxy.image_generation.transformation import (
    LiteLLMProxyImageGenerationConfig,
)
from litellm.llms.litellm_proxy.responses.transformation import LiteLLMProxyResponsesAPIConfig
from litellm.types.router import LiteLLM_Params
from litellm.types.utils import LlmProviders
from pydantic import ConfigDict, JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
MANAGED_ENV: Final = (
    "LITELLM_PROXY_API_BASE",
    "LITELLM_PROXY_API_KEY",
    "USE_LITELLM_PROXY",
    "OPENAI_API_KEY",
)
VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# every proxy image config ignores the model argument
IMAGE_MODEL: Final = "gpt-image-1"
GenerationEnvironment: TypeAlias = Callable[
    [dict[str, str], str, list[JsonValue], Object, Object, str | None, str | None],
    dict[str, str],
]
GenerationUrl: TypeAlias = Callable[
    [str | None, str | None, str, Object, Object, bool | None], str
]
EditEnvironment: TypeAlias = Callable[
    [dict[str, str], str, str | None, Object | None, str | None], dict[str, str]
]


@runtime_checkable
class ChatConfig(Protocol):
    def map_openai_params(
        self, non_default_params: Object, optional_params: Object, model: str, *, drop_params: bool
    ) -> Object: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: Object,
        litellm_params: Object,
        headers: dict[str, str],
    ) -> Object: ...


@runtime_checkable
class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    def get_complete_url(self, api_base: str | None, litellm_params: Object) -> str: ...

    def supports_native_websocket(self) -> bool: ...


@runtime_checkable
class GenerationConfig(Protocol):
    @property
    def validate_environment(self) -> GenerationEnvironment: ...

    @property
    def get_complete_url(self) -> GenerationUrl: ...


@runtime_checkable
class EditConfig(Protocol):
    @property
    def validate_environment(self) -> EditEnvironment: ...

    @property
    def get_complete_url(self) -> Callable[[str, str | None, Object], str]: ...


class ResponsesView(LiteLLMProxyResponsesAPIConfig):
    def __init__(self) -> None: ...


class EditView(LiteLLMProxyImageEditConfig):
    def __init__(self) -> None: ...


ARBITRARY: Final[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)
CONFIG: Final = LiteLLMProxyChatConfig()
# the pinned method has no return annotation; the declared Callable keeps the Any out of every call
FORCE_DEFAULT: Callable[[LiteLLM_Params | None], bool] = (
    CONFIG.should_use_litellm_proxy_by_default
)
CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(CONFIG)
RESPONSES: ResponsesConfig = TypeAdapter(ResponsesConfig, config=ARBITRARY).validate_python(
    ResponsesView()
)
GENERATION: GenerationConfig = TypeAdapter(GenerationConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyImageGenerationConfig()
)
EDIT: EditConfig = TypeAdapter(EditConfig, config=ARBITRARY).validate_python(EditView())


class RouteCase(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    environment_use_litellm_proxy: bool | None
    global_use_litellm_proxy: bool
    deployment_use_litellm_proxy: bool


class ChatCase(TypedDict):
    model: str
    messages: list[JsonValue]
    non_default: Object
    optional_params: Object


class ResponsesCase(TypedDict):
    api_base: str | None
    environment_base: str | None


class ImageCase(TypedDict):
    endpoint: Literal["generations", "edits"]
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    headers: dict[str, str]


class ModelsCase(TypedDict):
    api_base: str | None
    api_key: str | None
    environment_base: str | None
    environment_key: str | None
    openai_api_key: str | None
    reply_status: int
    reply_body: str


class Input(TypedDict):
    operation: Literal["route", "chat_request", "responses_url", "image_request", "models"]
    params: JsonValue


@dataclass
class Recorder:
    status: int
    body: str
    url: str | None = None
    authorization: str | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.authorization = request.headers.get("authorization")
        return httpx.Response(self.status, text=self.body, request=request)


def install_environment(
    *,
    base: str | None,
    key: str | None,
    forced: bool | None,
    openai_key: str | None,
) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    forced_text = None if forced is None else ("true" if forced else "false")
    values: dict[str, str | None] = {
        "LITELLM_PROXY_API_BASE": base,
        "LITELLM_PROXY_API_KEY": key,
        "USE_LITELLM_PROXY": forced_text,
        "OPENAI_API_KEY": openai_key,
    }
    os.environ.update({name: text for name, text in values.items() if text is not None})


def route_value(case: RouteCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=case["environment_use_litellm_proxy"],
        openai_key=None,
    )
    litellm.use_litellm_proxy = case["global_use_litellm_proxy"]
    params = LiteLLM_Params(
        model=case["model"], use_litellm_proxy=case["deployment_use_litellm_proxy"]
    )
    model, provider, api_key, api_base = CONFIG.litellm_proxy_get_custom_llm_provider_info(
        case["model"], case["api_base"], case["api_key"]
    )
    forced: bool = FORCE_DEFAULT(params)
    return {
        "should_use": forced,
        "model": model,
        "provider": provider,
        "api_base": api_base,
        "api_key": api_key,
        "get_api_key": CONFIG.get_api_key(case["api_key"]),
    }


def chat_value(case: ChatCase) -> JsonValue:
    optional = dict(case["optional_params"])
    mapped = CHAT.map_openai_params(
        dict(case["non_default"]), optional, case["model"], drop_params=False
    )
    request = CHAT.transform_request(case["model"], case["messages"], mapped, {}, {})
    return {"mapped": mapped, "request": request}


def responses_value(case: ResponsesCase) -> JsonValue:
    install_environment(base=case["environment_base"], key=None, forced=None, openai_key=None)
    try:
        url = RESPONSES.get_complete_url(case["api_base"], {})
    except ValueError:
        return {"ok": False}
    provider = RESPONSES.custom_llm_provider.value
    websocket = RESPONSES.supports_native_websocket()
    return {"ok": True, "url": url, "provider": provider, "websocket": websocket}


def image_transport(case: ImageCase) -> tuple[str, dict[str, str]]:
    headers: dict[str, str] = dict(case["headers"])
    if case["endpoint"] == "generations":
        url = GENERATION.get_complete_url(
            case["api_base"], case["api_key"], IMAGE_MODEL, {}, {}, None
        )
        return url, GENERATION.validate_environment(
            headers, IMAGE_MODEL, [], {}, {}, case["api_key"], case["api_base"]
        )
    url = EDIT.get_complete_url(IMAGE_MODEL, case["api_base"], {})
    return url, EDIT.validate_environment(
        headers, IMAGE_MODEL, case["api_key"], {}, case["api_base"]
    )


def image_value(case: ImageCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=None,
        openai_key=None,
    )
    try:
        url, headers = image_transport(case)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "url": url, "headers": VALUE.validate_python(headers)}


def models_value(case: ModelsCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=None,
        openai_key=case["openai_api_key"],
    )
    recorder = Recorder(status=case["reply_status"], body=case["reply_body"])
    client = httpx.Client(transport=httpx.MockTransport(recorder.handle))
    litellm.module_level_client = HTTPHandler(client=client)
    try:
        models = CONFIG.get_models(api_key=case["api_key"], api_base=case["api_base"])
    except ValueError:
        return {"ok": False}
    return {
        "ok": True,
        "models": VALUE.validate_python(models),
        "url": recorder.url,
        "authorization": recorder.authorization,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "route":
        cases = TypeAdapter(list[RouteCase]).validate_python(input_value["params"])
        return {"results": [route_value(case) for case in cases]}
    if input_value["operation"] == "chat_request":
        chat_cases = TypeAdapter(list[ChatCase]).validate_python(input_value["params"])
        return {"results": [chat_value(case) for case in chat_cases]}
    if input_value["operation"] == "responses_url":
        response_cases = TypeAdapter(list[ResponsesCase]).validate_python(input_value["params"])
        return {"results": [responses_value(case) for case in response_cases]}
    if input_value["operation"] == "image_request":
        image_cases = TypeAdapter(list[ImageCase]).validate_python(input_value["params"])
        return {"results": [image_value(case) for case in image_cases]}
    model_cases = TypeAdapter(list[ModelsCase]).validate_python(input_value["params"])
    return {"results": [models_value(case) for case in model_cases]}

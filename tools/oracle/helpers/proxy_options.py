import os
from collections.abc import Callable
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

MANAGED_ENV: Final = (
    "LITELLM_PROXY_API_BASE",
    "LITELLM_PROXY_API_KEY",
    "USE_LITELLM_PROXY",
    "OPENAI_API_KEY",
)
VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

TransformRequest: TypeAlias = Callable[
    [str, list[JsonValue], dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
    dict[str, JsonValue],
]
GenerationEnvironment: TypeAlias = Callable[
    [
        dict[str, str],
        str,
        list[JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
        str | None,
    ],
    dict[str, str],
]
GenerationUrl: TypeAlias = Callable[
    [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue], bool | None],
    str,
]
EditEnvironment: TypeAlias = Callable[
    [dict[str, str], str, str | None, dict[str, JsonValue] | None, str | None],
    dict[str, str],
]
EditUrl: TypeAlias = Callable[[str, str | None, dict[str, JsonValue]], str]


@runtime_checkable
class ChatConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def get_models(self, api_key: str | None = None, api_base: str | None = None) -> list[str]: ...

    def get_api_key(self, api_key: str | None = None) -> str | None: ...

    def should_use_litellm_proxy_by_default(  # ast-grep-ignore: no-long-names
        self, litellm_params: LiteLLM_Params | None = None
    ) -> bool: ...

    def litellm_proxy_get_custom_llm_provider_info(  # ast-grep-ignore: no-long-names
        self, model: str, api_base: str | None = None, api_key: str | None = None
    ) -> tuple[str, str, str | None, str | None]: ...


@runtime_checkable
class ResponsesConfig(Protocol):
    @property
    def custom_llm_provider(self) -> LlmProviders: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

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
    def get_complete_url(self) -> EditUrl: ...


ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)

CHAT: ChatConfig = TypeAdapter(ChatConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyChatConfig()
)
RESPONSES: ResponsesConfig = TypeAdapter(ResponsesConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyResponsesAPIConfig()
)
GENERATION: GenerationConfig = TypeAdapter(
    GenerationConfig, config=ARBITRARY
).validate_python(LiteLLMProxyImageGenerationConfig())
EDIT: EditConfig = TypeAdapter(EditConfig, config=ARBITRARY).validate_python(
    LiteLLMProxyImageEditConfig()
)


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
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class ResponsesCase(TypedDict):
    api_base: str | None
    environment_base: str | None


class ImageCase(TypedDict):
    endpoint: Literal["generations", "edits"]
    model: str
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


class Recorder:
    def __init__(self, *, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.url: str | None = None
        self.authorization: str | None = None

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
    values: dict[str, str | None] = {
        "LITELLM_PROXY_API_BASE": base,
        "LITELLM_PROXY_API_KEY": key,
        "OPENAI_API_KEY": openai_key,
    }
    for name, value in values.items():
        if value is not None:
            os.environ[name] = value
    if forced is not None:
        os.environ["USE_LITELLM_PROXY"] = "true" if forced else "false"


def route_value(case: RouteCase) -> JsonValue:
    install_environment(
        base=case["environment_base"],
        key=case["environment_key"],
        forced=case["environment_use_litellm_proxy"],
        openai_key=None,
    )
    litellm.use_litellm_proxy = case["global_use_litellm_proxy"]
    params = LiteLLM_Params(
        model=case["model"],
        use_litellm_proxy=case["deployment_use_litellm_proxy"],
    )
    should_use = CHAT.should_use_litellm_proxy_by_default(params)
    model, provider, api_key, api_base = CHAT.litellm_proxy_get_custom_llm_provider_info(
        case["model"], case["api_base"], case["api_key"]
    )
    return {
        "should_use": should_use,
        "model": model,
        "provider": provider,
        "api_base": api_base,
        "api_key": api_key,
        "get_api_key": CHAT.get_api_key(case["api_key"]),
    }


def chat_value(case: ChatCase) -> JsonValue:
    optional = dict(case["optional_params"])
    mapped = CHAT.map_openai_params(
        dict(case["non_default"]),
        optional,
        case["model"],
        drop_params=False,
    )
    request = CHAT.transform_request(
        case["model"],
        case["messages"],
        mapped,
        {},
        {},
    )
    return {"mapped": mapped, "request": request}


def responses_value(case: ResponsesCase) -> JsonValue:
    install_environment(
        base=case["environment_base"], key=None, forced=None, openai_key=None
    )
    try:
        url = RESPONSES.get_complete_url(case["api_base"], {})
    except ValueError:
        return {"ok": False}
    return {
        "ok": True,
        "url": url,
        "provider": RESPONSES.custom_llm_provider.value,
        "websocket": RESPONSES.supports_native_websocket(),
    }


def image_transport(case: ImageCase) -> tuple[str, dict[str, str]]:
    headers: dict[str, str] = dict(case["headers"])
    if case["endpoint"] == "generations":
        url = GENERATION.get_complete_url(
            case["api_base"], case["api_key"], case["model"], {}, {}, None
        )
        return url, GENERATION.validate_environment(
            headers, case["model"], [], {}, {}, case["api_key"], case["api_base"]
        )
    url = EDIT.get_complete_url(case["model"], case["api_base"], {})
    return url, EDIT.validate_environment(
        headers, case["model"], case["api_key"], {}, case["api_base"]
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
        models = CHAT.get_models(api_key=case["api_key"], api_base=case["api_base"])
    except ValueError:
        return {"ok": False}
    return {
        "ok": True,
        "models": VALUE.validate_python(models),
        "url": recorder.url,
        "authorization": recorder.authorization,
    }


def route_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[RouteCase]).validate_python(params)
    return {"results": [route_value(case) for case in cases]}


def chat_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ChatCase]).validate_python(params)
    return {"results": [chat_value(case) for case in cases]}


def responses_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ResponsesCase]).validate_python(params)
    return {"results": [responses_value(case) for case in cases]}


def image_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ImageCase]).validate_python(params)
    return {"results": [image_value(case) for case in cases]}


def models_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ModelsCase]).validate_python(params)
    return {"results": [models_value(case) for case in cases]}


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    params = input_value["params"]
    if operation == "route":
        return route_cases(params)
    if operation == "chat_request":
        return chat_cases(params)
    if operation == "responses_url":
        return responses_cases(params)
    if operation == "image_request":
        return image_cases(params)
    return models_cases(params)

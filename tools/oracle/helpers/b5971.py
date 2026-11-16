import os
from collections.abc import Callable, Coroutine
from contextlib import suppress
from typing import NotRequired, Protocol, TypeAlias, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.watsonx.chat.handler import WatsonXChatHandler
from litellm.llms.watsonx.chat.transformation import IBMWatsonXChatConfig
from litellm.types.utils import CustomStreamingDecoder, ModelResponse
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
Completion: TypeAlias = (
    CustomStreamWrapper
    | ModelResponse
    | Coroutine[None, None, CustomStreamWrapper | ModelResponse]
)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "watsonx-model"
VARIABLES = (
    "WATSONX_IAM_URL",
    "WX_API_KEY",
    "WATSONX_API_KEY",
    "WATSONX_APIKEY",
    "WATSONX_ZENAPIKEY",
    "WATSONX_TOKEN",
    "WATSONX_API_BASE",
    "WATSONX_URL",
    "WX_URL",
    "WML_URL",
    "WATSONX_PROJECT_ID",
    "WX_PROJECT_ID",
    "PROJECT_ID",
    "WATSONX_REGION",
    "WX_REGION",
    "REGION",
    "WATSONX_DEPLOYMENT_SPACE_ID",
    "WATSONX_SPACE_ID",
    "WX_SPACE_ID",
    "SPACE_ID",
    "HOSTED_VLLM_API_BASE",
    "HOSTED_VLLM_API_KEY",
)


class MapCase(TypedDict):
    non_default: JsonObject
    optional: JsonObject


class InfoCase(TypedDict):
    api_base: str | None
    api_key: str | None


class UrlCase(TypedDict):
    api_base: str | None
    model: str
    stream: bool
    options: JsonObject


class PlanCase(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str]
    options: JsonObject


class Input(TypedDict):
    env: dict[str, str]
    maps: list[MapCase]
    infos: list[InfoCase]
    urls: list[UrlCase]
    plans: list[PlanCase]


class AdditionalArguments(TypedDict):
    complete_input_dict: JsonObject
    api_base: str
    headers: dict[str, str]


class PreCallArguments(TypedDict):
    input: list[JsonObject]
    api_key: str | None
    additional_args: AdditionalArguments


class StoppedError(Exception):
    pass


# stands in for the logging object, whose pre_call sees the request OpenAILikeChatHandler would send
class Recorder:
    def __init__(self) -> None:
        self.sent: list[AdditionalArguments] = []

    def pre_call(self, **kwargs: Unpack[PreCallArguments]) -> None:
        self.sent.append(kwargs["additional_args"])
        raise StoppedError


class CallArguments(TypedDict):
    model: str
    messages: list[JsonObject]
    api_base: str | None
    custom_llm_provider: str
    custom_prompt_dict: JsonObject
    model_response: ModelResponse
    print_verbose: Callable[[str], None]
    encoding: None
    api_key: str | None
    logging_obj: Recorder
    optional_params: JsonObject
    acompletion: NotRequired[bool | None]
    litellm_params: NotRequired[JsonObject]
    logger_fn: NotRequired[None]
    headers: NotRequired[dict[str, str] | None]
    timeout: NotRequired[float | httpx.Timeout | None]
    client: NotRequired[HTTPHandler | AsyncHTTPHandler | None]
    custom_endpoint: NotRequired[bool | None]
    streaming_decoder: NotRequired[CustomStreamingDecoder | None]
    fake_stream: NotRequired[bool]


class UrlArguments(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    optional_params: JsonObject
    litellm_params: JsonObject
    stream: NotRequired[bool | None]


class ChatApi(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_complete_url(self, **kwargs: Unpack[UrlArguments]) -> str: ...

    def provider_info(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str | None, str | None]: ...


class Completer(Protocol):
    def completion(self, **kwargs: Unpack[CallArguments]) -> Completion: ...


class Config(IBMWatsonXChatConfig):
    def provider_info(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str | None, str | None]:
        return self._get_openai_compatible_provider_info(api_base, api_key)


class Probe(WatsonXChatHandler):
    def __init__(self) -> None:
        pass


CONFIG: ChatApi = Config()
HANDLER: Completer = Probe()


def run_map(case: MapCase) -> JsonValue:
    mapped = CONFIG.map_openai_params(
        non_default_params=dict(case["non_default"]),
        optional_params=dict(case["optional"]),
        model=MODEL,
        drop_params=False,
    )
    return {"optional": mapped}


def run_info(case: InfoCase) -> JsonValue:
    api_base, api_key = CONFIG.provider_info(case["api_base"], case["api_key"])
    return {"api_base": api_base, "api_key": api_key}


def run_url(case: UrlCase) -> JsonValue:
    options = dict(case["options"])
    url = CONFIG.get_complete_url(
        api_base=case["api_base"],
        api_key=None,
        model=case["model"],
        optional_params=options,
        litellm_params={},
        stream=case["stream"],
    )
    return {"url": url, "options": options}


def run_plan(case: PlanCase) -> JsonValue:
    recorder = Recorder()
    with suppress(StoppedError):
        HANDLER.completion(
            model=case["model"],
            messages=[],
            api_base=case["api_base"],
            custom_llm_provider="watsonx",
            custom_prompt_dict={},
            model_response=ModelResponse(),
            print_verbose=print,
            encoding=None,
            api_key=case["api_key"],
            logging_obj=recorder,
            optional_params=dict(case["options"]),
            headers=case["headers"],
        )
    sent = recorder.sent[0]
    data = sent["complete_input_dict"]
    options: JsonObject = {
        name: value
        for name, value in data.items()
        if name not in {"model", "messages"}
    }
    headers: JsonObject = {name.lower(): value for name, value in sent["headers"].items()}
    return {
        "plan": {
            "model": data["model"],
            "url": sent["api_base"],
            "headers": headers,
            "options": options,
        }
    }


def run(input_value: Input) -> JsonValue:
    saved = {name: os.environ[name] for name in VARIABLES if name in os.environ}
    for name in VARIABLES:
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    try:
        maps = [run_map(case) for case in input_value["maps"]]
        infos = [run_info(case) for case in input_value["infos"]]
        urls = [run_url(case) for case in input_value["urls"]]
        plans = [run_plan(case) for case in input_value["plans"]]
        supported = JSON.validate_python(CONFIG.get_supported_openai_params(MODEL))
    finally:
        for name in VARIABLES:
            os.environ.pop(name, None)
        os.environ.update(saved)
    return {
        "supported": supported,
        "maps": maps,
        "infos": infos,
        "urls": urls,
        "plans": plans,
    }

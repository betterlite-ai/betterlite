import os
from typing import Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

MODEL = "eleven_multilingual_v2"
ENVIRONMENT_NAMES = ("ELEVENLABS_API_KEY", "ELEVENLABS_API_BASE")


# LiteLLM declares its kwargs as this, and a dict of JSON values is not one
Kwargs: TypeAlias = dict[str, object]


class SpeechRequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]
    headers: dict[str, str]


class Config(Protocol):
    def map_openai_params(
        self,
        model: str,
        optional_params: dict[str, JsonValue],
        voice: str | dict[str, JsonValue] | None,
        *,
        drop_params: bool,
        kwargs: Kwargs,
    ) -> tuple[str | None, dict[str, JsonValue]]: ...

    def transform_text_to_speech_request(
        self,
        model: str,
        text: str,
        voice: str | None,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> SpeechRequestData: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None,
        api_base: str | None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self,
        model: str,
        api_base: str | None,
        litellm_params: dict[str, JsonValue],
    ) -> str: ...


# LiteLLM leaves the options and kwargs bare, so this typed view is the public entry to the config
PROVIDER_CONFIG = ProviderConfigManager.get_provider_text_to_speech_config(
    model=MODEL, provider=LlmProviders.ELEVENLABS
)
if PROVIDER_CONFIG is None:
    raise RuntimeError("LiteLLM has no ElevenLabs text-to-speech config")
CONFIG: Config = PROVIDER_CONFIG


class MapCall(TypedDict):
    function: Literal["map_params"]
    voice: str | dict[str, JsonValue] | None
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]


class RequestCall(TypedDict):
    function: Literal["request"]
    text: str
    optional_params: dict[str, JsonValue]


class UrlCall(TypedDict):
    function: Literal["url"]
    api_base: str | None
    environment: dict[str, str]
    litellm_params: dict[str, JsonValue]


class HeadersCall(TypedDict):
    function: Literal["headers"]
    headers: dict[str, str]
    api_key: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    environment: dict[str, str]


Call: TypeAlias = MapCall | RequestCall | UrlCall | HeadersCall


class Input(TypedDict):
    calls: list[Call]


class RequestBody(TypedDict):
    dict_body: dict[str, JsonValue]
    headers: dict[str, str]


class Mapped(TypedDict):
    ok: bool
    voice_id: str | None
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]


class Requested(TypedDict):
    ok: bool
    dict_body: dict[str, JsonValue]
    headers: dict[str, str]


class Located(TypedDict):
    ok: bool
    url: str


class Authenticated(TypedDict):
    ok: bool
    headers: dict[str, str]


class Failure(TypedDict):
    ok: bool


Answer: TypeAlias = Mapped | Requested | Located | Authenticated | Failure


class Output(TypedDict):
    calls: list[Answer]


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def map_answer(call: MapCall) -> Mapped:
    optional_params = call["optional_params"]
    kwargs: Kwargs = {**call["kwargs"]}
    voice_id, params = CONFIG.map_openai_params(
        model=MODEL,
        optional_params=optional_params,
        voice=call["voice"],
        drop_params=False,
        kwargs=kwargs,
    )
    return {
        "ok": True,
        "voice_id": voice_id,
        "params": params,
        "optional_params": optional_params,
        "kwargs": TypeAdapter(dict[str, JsonValue]).validate_python(kwargs),
    }


def request_answer(call: RequestCall) -> Requested:
    data = CONFIG.transform_text_to_speech_request(MODEL, call["text"], None, call["optional_params"], {}, {})
    body = TypeAdapter(RequestBody).validate_python(data)
    return {"ok": True, "dict_body": body["dict_body"], "headers": body["headers"]}


def url_answer(call: UrlCall) -> Located:
    apply_environment(call["environment"])
    url = CONFIG.get_complete_url(
        model=MODEL, api_base=call["api_base"], litellm_params=call["litellm_params"]
    )
    return {"ok": True, "url": url}


def headers_answer(call: HeadersCall) -> Authenticated:
    apply_environment(call["environment"])
    litellm.api_key = call["litellm_api_key"]
    litellm.openai_key = call["litellm_openai_key"]
    headers = CONFIG.validate_environment(
        headers=call["headers"], model=MODEL, api_key=call["api_key"], api_base=None
    )
    return {"ok": True, "headers": headers}


def answer(call: Call) -> Answer:
    try:
        if call["function"] == "map_params":
            return map_answer(call)
        if call["function"] == "request":
            return request_answer(call)
        if call["function"] == "url":
            return url_answer(call)
        return headers_answer(call)
    except ValueError:
        return {"ok": False}


def run(input_value: Input) -> Output:
    answers: list[Answer] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}

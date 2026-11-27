import base64
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
import litellm.llms.runwayml.text_to_speech.transformation as runway_transformation
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.text_to_speech.transformation import BaseTextToSpeechConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import ConfigDict, JsonValue, TypeAdapter

MODEL = "eleven_multilingual_v2"
SUBMISSION_URL = "https://submission.example.test/v1/text_to_speech"
EMPTY: dict[str, JsonValue] = {}
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

Headers: TypeAlias = dict[str, str]
Object: TypeAlias = dict[str, JsonValue]
Voice: TypeAlias = str | dict[str, JsonValue] | None


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: HTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)
PARAMS: TypeAdapter[Object] = TypeAdapter(Object)


class SpeechRequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]
    ssml_body: str
    headers: dict[str, str]


class Handler(Protocol):
    def text_to_speech_handler(self, **kwargs: JsonValue) -> JsonValue: ...


Supported: TypeAlias = Callable[[str], Sequence[str]]
Mapper: TypeAlias = Callable[[str, Object, Voice, bool, Object], tuple[str | None, Object]]
Requester: TypeAlias = Callable[[str, str, Voice, Object, Object, Headers], SpeechRequestData]
Responder: TypeAlias = Callable[[str, httpx.Response, Logging], HttpxBinaryResponseContent]
Dispatcher: TypeAlias = Callable[
    [
        str,
        str,
        Voice,
        Object,
        Object,
        Logging,
        float,
        Object | None,
        Handler,
        bool,
        str | None,
        str | None,
    ],
    HttpxBinaryResponseContent,
]


@runtime_checkable
class SpeechConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def map_openai_params(self) -> Mapper: ...

    @property
    def transform_text_to_speech_request(self) -> Requester: ...  # ast-grep-ignore: no-long-names

    @property
    def transform_text_to_speech_response(self) -> Responder: ...  # ast-grep-ignore: no-long-names

    @property
    def dispatch_text_to_speech(self) -> Dispatcher: ...


BASE: BaseTextToSpeechConfig | None = ProviderConfigManager.get_provider_text_to_speech_config(
    model=MODEL, provider=LlmProviders.RUNWAYML
)
if BASE is None:
    raise RuntimeError("LiteLLM has no RunwayML text-to-speech config")
SPEECH: TypeAdapter[SpeechConfig] = TypeAdapter(
    SpeechConfig, config=ConfigDict(arbitrary_types_allowed=True)
)
CONFIG: SpeechConfig = SPEECH.validate_python(BASE)


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None


class MappingCall(TypedDict):
    voice: Voice
    optional_params: Object
    drop_params: bool


class MappingParams(TypedDict):
    model: str
    environment: dict[str, str]
    calls: list[MappingCall]


class MappingInput(TypedDict):
    operation: Literal["mapping"]
    params: MappingParams


class RequestParams(TypedDict):
    model: str
    input: str
    optional_params: Object
    headers: Headers


class RequestInput(TypedDict):
    operation: Literal["request"]
    params: RequestParams


class DispatchParams(TypedDict):
    model: str
    input: str
    voice: Voice
    optional_params: Object
    litellm_params: Object
    api_base: str | None
    api_key: str | None
    timeout_seconds: float
    extra_headers: Object | None
    aspeech: bool
    environment: dict[str, str]
    globals: Globals


class DispatchInput(TypedDict):
    operation: Literal["dispatch"]
    params: DispatchParams


class Submission(TypedDict):
    status: int
    headers: Headers
    body: str


class Reply(TypedDict):
    status: int
    headers: Headers
    body: str


class ResponseParams(TypedDict):
    model: str
    environment: dict[str, str]
    submission: Submission
    request_headers: Headers
    step_millis: int
    replies: list[Reply]


class ResponseInput(TypedDict):
    operation: Literal["response"]
    params: ResponseParams


Input: TypeAlias = MappingInput | RequestInput | DispatchInput | ResponseInput


class Clock:
    def __init__(self, step_millis: int) -> None:
        self.millis = 0
        self.step_millis = step_millis
        self.sleeps: list[JsonValue] = []

    def time(self) -> float:
        current = self.millis
        self.millis += self.step_millis
        return current / 1000.0

    def sleep(self, seconds: float) -> None:
        pause = int(seconds)
        self.sleeps.append(pause)


class Scripted:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if not self.replies:
            return httpx.Response(500, content=b"")
        reply = self.replies.pop(0)
        return httpx.Response(
            reply["status"], headers=reply["headers"], content=reply["body"].encode()
        )


class Recorder:
    def __init__(self) -> None:
        self.call: Object = {}

    def text_to_speech_handler(self, **kwargs: JsonValue) -> JsonValue:
        params = PARAMS.validate_python(kwargs["litellm_params"])
        call: Object = {
            "model": kwargs["model"],
            "input": kwargs["input"],
            "voice": kwargs["voice"],
            "optional_params": kwargs["text_to_speech_optional_params"],
            "litellm_params": params,
            "api_base": params["api_base"],
            "api_key": params["api_key"],
            "timeout_seconds": kwargs["timeout"],
            "extra_headers": kwargs["extra_headers"],
            "aspeech": kwargs["_is_async"],
            "client": kwargs["client"],
        }
        self.call = call
        return call


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="speech",
        start_time=datetime.now(UTC),
        litellm_call_id="runway-speech-call",
        function_id="runway-speech-function",
    )


def header_text(headers: httpx.Headers, name: str) -> str:
    value = headers.get(name)
    if value is None:
        return ""
    return value


def request_value(request: httpx.Request) -> JsonValue:
    headers: Object = {
        "authorization": header_text(request.headers, "authorization"),
        "x-runway-version": header_text(request.headers, "x-runway-version"),
    }
    answer: Object = {"url": str(request.url), "headers": headers}
    return JSON.validate_python(answer)


def response_value(finished: HttpxBinaryResponseContent) -> JsonValue:
    reply = finished.response
    answer: Object = {
        "status": reply.status_code,
        "content_type": reply.headers.get("content-type"),
        "body_b64": base64.b64encode(reply.content).decode("ascii"),
    }
    return JSON.validate_python(answer)


def mapping_value(input_value: MappingInput) -> JsonValue:
    params = input_value["params"]
    results: list[JsonValue] = []
    for call in params["calls"]:
        voice, mapped = CONFIG.map_openai_params(
            params["model"], call["optional_params"], call["voice"], call["drop_params"], EMPTY
        )
        results.append({"voice": voice, "params": mapped})
    answer: Object = {
        "supported": JSON.validate_python(CONFIG.get_supported_openai_params(params["model"])),
        "results": results,
    }
    return JSON.validate_python(answer)


def request_answer(input_value: RequestInput) -> JsonValue:
    params = input_value["params"]
    data = CONFIG.transform_text_to_speech_request(
        params["model"], params["input"], None, params["optional_params"], EMPTY, params["headers"]
    )
    answer: Object = {"dict_body": data["dict_body"], "headers": data["headers"]}
    return JSON.validate_python(answer)


def dispatch_value(input_value: DispatchInput) -> JsonValue:
    params = input_value["params"]
    globals_value = params["globals"]
    litellm.api_base = globals_value["api_base"]
    litellm.api_key = globals_value["api_key"]
    recorder = Recorder()
    CONFIG.dispatch_text_to_speech(
        params["model"],
        params["input"],
        params["voice"],
        params["optional_params"],
        dict(params["litellm_params"]),
        logging_object(params["model"]),
        params["timeout_seconds"],
        params["extra_headers"],
        recorder,
        params["aspeech"],
        params["api_base"],
        params["api_key"],
    )
    return JSON.validate_python(recorder.call)


def transform_value(input_value: ResponseInput) -> JsonValue:
    params = input_value["params"]
    scripted = Scripted(params["replies"])
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(scripted)))
    CLIENT_CACHE.validate_python(holder).set_cache(key="httpx_client", value=client, ttl=600)
    clock = Clock(params["step_millis"])
    object.__setattr__(runway_transformation, "time", clock)
    submission = params["submission"]
    request = httpx.Request("POST", SUBMISSION_URL, headers=params["request_headers"])
    raw_response = httpx.Response(
        submission["status"],
        headers=submission["headers"],
        content=submission["body"].encode(),
        request=request,
    )
    finished = CONFIG.transform_text_to_speech_response(
        params["model"], raw_response, logging_object(params["model"])
    )
    gets: list[JsonValue] = [request_value(item) for item in scripted.requests]
    answer: Object = {
        "response": response_value(finished),
        "gets": gets,
        "sleeps": clock.sleeps,
    }
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "mapping":
        return mapping_value(input_value)
    if input_value["operation"] == "request":
        return request_answer(input_value)
    if input_value["operation"] == "dispatch":
        return dispatch_value(input_value)
    return transform_value(input_value)

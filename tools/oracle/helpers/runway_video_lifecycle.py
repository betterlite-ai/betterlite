from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.types.videos.main import VideoObject
from litellm.utils import ProviderConfigManager
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
EMPTY: Final[JsonObject] = {}
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
API_BASE: Final[str] = "https://api.runwayml.test/v1"
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)

TaskRequest = Callable[[str, str, GenericLiteLLMParams, JsonObject], tuple[str, JsonObject]]
ContentRequest = Callable[
    [str, str, GenericLiteLLMParams, JsonObject, str | None], tuple[str, JsonObject]
]
VideoReply = Callable[[httpx.Response, Logging, str | None], VideoObject]


class UnsupportedCall(Protocol):
    def __call__(self, *args: str | GenericLiteLLMParams | JsonObject) -> JsonValue: ...


@runtime_checkable
class VideoConfig(Protocol):
    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: JsonObject
    ) -> str: ...

    @property
    def transform_video_content_request(self) -> ContentRequest: ...  # ast-grep-ignore

    @property
    def transform_video_status_retrieve_request(self) -> TaskRequest: ...  # ast-grep-ignore

    @property
    def transform_video_delete_request(self) -> TaskRequest: ...

    @property
    def transform_video_status_retrieve_response(self) -> VideoReply: ...  # ast-grep-ignore

    @property
    def transform_video_delete_response(  # ast-grep-ignore
        self,
    ) -> Callable[[httpx.Response, Logging], VideoObject]: ...

    @property
    def transform_video_content_response(  # ast-grep-ignore
        self,
    ) -> Callable[[httpx.Response, Logging], bytes]: ...

    @property
    def transform_video_remix_request(self) -> UnsupportedCall: ...

    @property
    def transform_video_list_request(self) -> UnsupportedCall: ...

    @property
    def transform_video_create_character_request(self) -> UnsupportedCall: ...  # ast-grep-ignore

    @property
    def transform_video_get_character_request(self) -> UnsupportedCall: ...  # ast-grep-ignore

    @property
    def transform_video_edit_request(self) -> UnsupportedCall: ...

    @property
    def transform_video_extension_request(self) -> UnsupportedCall: ...  # ast-grep-ignore


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: HTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: Final[TypeAdapter[ClientCache]] = TypeAdapter(ClientCache, config=ARBITRARY)


def runway_config() -> VideoConfig:
    provider = ProviderConfigManager.get_provider_video_config(None, LlmProviders.RUNWAYML)
    if provider is None:
        raise ValueError("the RunwayML video config is missing")
    return TypeAdapter[VideoConfig](VideoConfig, config=ARBITRARY).validate_python(provider)


CONFIG: Final[VideoConfig] = runway_config()


class RequestCall(TypedDict):
    kind: Literal["content", "status", "delete"]
    video_id: str
    api_base: str | None
    variant: str | None


class RequestInput(TypedDict):
    operation: Literal["request"]
    params: list[RequestCall]


class ResponseCall(TypedDict):
    kind: Literal["retrieve", "delete"]
    provider: str | None
    body: str


class ResponseInput(TypedDict):
    operation: Literal["response"]
    params: list[ResponseCall]


class Reply(TypedDict):
    status: int
    body: str


class ContentCall(TypedDict):
    body: str
    reply: Reply


class ContentInput(TypedDict):
    operation: Literal["content"]
    params: list[ContentCall]


UnsupportedName = Literal["remix", "list", "create_character", "get_character", "edit", "extension"]


class UnsupportedInput(TypedDict):
    operation: Literal["unsupported"]
    params: list[UnsupportedName]


class Scripted:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(self.reply["status"], content=self.reply["body"].encode())


def logging_object() -> Logging:
    return Logging(
        model="runwayml",
        messages=None,
        stream=False,
        call_type="video_generation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="runway-video-lifecycle",
        function_id="runway-video-lifecycle",
    )


def install_client(scripted: Scripted) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(scripted)))
    CLIENT_CACHE.validate_python(holder).set_cache(key="httpx_client", value=client, ttl=600)


def artifact_value(requests: list[httpx.Request]) -> JsonValue:
    if not requests:
        return {"method": None, "url": None, "authorization": None, "runway_version": None}
    request = requests[0]
    answer: JsonObject = {
        "method": request.method,
        "url": str(request.url),
        "authorization": next(iter(request.headers.get_list("authorization")), None),
        "runway_version": next(iter(request.headers.get_list("x-runway-version")), None),
    }
    return JSON.validate_python(answer)


def request_value(call: RequestCall) -> JsonValue:
    complete = CONFIG.get_complete_url("", call["api_base"], EMPTY)
    params = GenericLiteLLMParams()
    if call["kind"] == "content":
        url, data = CONFIG.transform_video_content_request(
            call["video_id"], complete, params, EMPTY, call["variant"]
        )
    elif call["kind"] == "status":
        url, data = CONFIG.transform_video_status_retrieve_request(
            call["video_id"], complete, params, EMPTY
        )
    else:
        url, data = CONFIG.transform_video_delete_request(call["video_id"], complete, params, EMPTY)
    answer: JsonObject = {"url": url, "data": data}
    return JSON.validate_python(answer)


def response_value(call: ResponseCall) -> JsonValue:
    raw = httpx.Response(200, content=call["body"].encode())
    if call["kind"] == "delete":
        deleted = CONFIG.transform_video_delete_response(raw, logging_object())
        return JSON.validate_json(deleted.model_dump_json())
    video = CONFIG.transform_video_status_retrieve_response(raw, logging_object(), call["provider"])
    return JSON.validate_json(video.model_dump_json())


def content_value(call: ContentCall) -> JsonValue:
    scripted = Scripted(call["reply"])
    install_client(scripted)
    raw = httpx.Response(200, content=call["body"].encode())
    try:
        content = CONFIG.transform_video_content_response(raw, logging_object())
    except httpx.HTTPStatusError as error:
        failure: JsonObject = {
            "artifact_status": error.response.status_code,
            "artifact_body": error.response.text,
            "artifact": artifact_value(scripted.requests),
        }
        return JSON.validate_python(failure)
    except ValueError as error:
        rejection: JsonObject = {"error": str(error)}
        return JSON.validate_python(rejection)
    answer: JsonObject = {"content": content.hex(), "artifact": artifact_value(scripted.requests)}
    return JSON.validate_python(answer)


def unsupported_value(name: UnsupportedName) -> JsonValue:
    params = GenericLiteLLMParams()
    try:
        if name == "remix":
            CONFIG.transform_video_remix_request("task_1", "a prompt", API_BASE, params, EMPTY)
        elif name == "list":
            CONFIG.transform_video_list_request(API_BASE, params, EMPTY)
        elif name == "create_character":
            CONFIG.transform_video_create_character_request(
                "a name", "a video", API_BASE, params, EMPTY
            )
        elif name == "get_character":
            CONFIG.transform_video_get_character_request("character_1", API_BASE, params, EMPTY)
        elif name == "edit":
            CONFIG.transform_video_edit_request("a prompt", "task_1", API_BASE, params, EMPTY)
        else:
            CONFIG.transform_video_extension_request(
                "a prompt", "task_1", "5", API_BASE, params, EMPTY
            )
    except NotImplementedError as error:
        return str(error)
    raise ValueError(f"{name} is expected to be unsupported")


def request_results(input_value: RequestInput) -> JsonValue:
    urls = [request_value(call) for call in input_value["params"]]
    answer: JsonObject = {"urls": urls}
    return JSON.validate_python(answer)


def response_results(input_value: ResponseInput) -> JsonValue:
    videos = [response_value(call) for call in input_value["params"]]
    answer: JsonObject = {"videos": videos}
    return JSON.validate_python(answer)


def content_results(input_value: ContentInput) -> JsonValue:
    results = [content_value(call) for call in input_value["params"]]
    answer: JsonObject = {"results": results}
    return JSON.validate_python(answer)


def unsupported_results(input_value: UnsupportedInput) -> JsonValue:
    messages: list[JsonValue] = [unsupported_value(name) for name in input_value["params"]]
    answer: JsonObject = {"messages": messages}
    return JSON.validate_python(answer)


def run(
    input_value: RequestInput | ResponseInput | ContentInput | UnsupportedInput,
) -> JsonValue:
    if input_value["operation"] == "request":
        return request_results(input_value)
    if input_value["operation"] == "response":
        return response_results(input_value)
    if input_value["operation"] == "content":
        return content_results(input_value)
    return unsupported_results(input_value)

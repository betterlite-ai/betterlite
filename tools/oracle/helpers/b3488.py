import asyncio
import base64
from collections.abc import AsyncIterator, Coroutine, Iterator
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeVar, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.google_genai.transformation import (
    BaseGoogleGenAIGenerateContentConfig as GenAiConfig,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True
JSON: Final = TypeAdapter(JsonValue)
DETAILS: Final = TypeAdapter(dict[str, JsonValue])
EXTRA: Final = TypeAdapter(dict[str, object] | None)
START: Final = datetime(2026, 1, 1, tzinfo=UTC)
KEPT: Final = frozenset({"content-type", "x-goog-api-key"})
PROVIDERS: Final[dict[str, LlmProviders]] = {
    "gemini": LlmProviders.GEMINI,
    "vertex_ai": LlmProviders.VERTEX_AI,
}
Provider = Literal["gemini", "vertex_ai"]
Answer = TypeVar("Answer")


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Params(TypedDict, total=False):
    api_key: str
    api_base: str
    model_info: dict[str, JsonValue]


class Case(TypedDict):
    mode: Literal["sync", "async"]
    provider: Provider
    model: str
    contents: JsonValue
    config: dict[str, JsonValue]
    tools: JsonValue | None
    system_instruction: JsonValue | None
    litellm_params: Params
    extra_headers: dict[str, str] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    stream: bool
    reply: Reply


class Generated(Protocol):
    def model_dump_json(self) -> str: ...


class SyncEvents(Protocol):
    def __iter__(self) -> Iterator[bytes]: ...


class AsyncEvents(Protocol):
    def __aiter__(self) -> AsyncIterator[bytes]: ...


class Hidden(TypedDict):
    model_id: str
    api_base: str
    cache_key: str
    response_cost: str
    additional_headers: dict[str, JsonValue]


class StreamState(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    hidden: Hidden = Field(alias="_hidden_params")


class HttpReply(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int
    text: str


class GenAiCall(Protocol[Answer]):
    def __call__(
        self,
        *,
        model: str,
        contents: JsonValue,
        generate_content_provider_config: GenAiConfig,
        generate_content_config_dict: dict[str, JsonValue],
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        tools: JsonValue | None = None,
        custom_llm_provider: str = "gemini",
        # LiteLLM annotates these two as dict[str, object] | None, and the values are validated into it
        extra_headers: dict[str, object] | None = None,  # ast-grep-ignore: no-loose-types
        extra_body: dict[str, object] | None = None,  # ast-grep-ignore: no-loose-types
        timeout: float | None = None,
        _is_async: bool = False,
        client: HTTPHandler | AsyncHTTPHandler | None = None,
        stream: bool = False,
        litellm_metadata: None = None,
        system_instruction: JsonValue | None = None,
    ) -> Answer: ...


class Recorder:
    def __init__(self, case: Case) -> None:
        self.status = case["reply"]["status"]
        self.headers = case["reply"]["headers"]
        self.body = case["reply"]["body"]
        self.url = ""
        self.method = ""
        self.seen: dict[str, str] = {}
        self.seen_body: JsonValue = None
        names = set(KEPT)
        extra = case["extra_headers"]
        if extra is not None:
            names.update(name.lower() for name in extra)
        self.keep: frozenset[str] = frozenset(names)

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.method = request.method
        kept: dict[str, str] = {}
        for name, value in request.headers.items():
            if name.lower() in self.keep:
                kept[name.lower()] = value
        self.seen = kept
        self.seen_body = JSON.validate_json(request.content)
        response = httpx.Response(
            self.status,
            headers=self.headers,
            content=self.body.encode(),
            request=request,
        )
        response.headers = httpx.Headers(self.headers)
        return response

    def view(self) -> dict[str, JsonValue]:
        return {
            "url": self.url,
            "method": self.method,
            "headers": JSON.validate_python(self.seen),
            "body": self.seen_body,
        }


def logging_object(case: Case) -> Logging:
    return Logging(
        model=case["model"],
        messages=None,
        stream=case["stream"],
        call_type="generate_content",
        start_time=START,
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id="trace-1",
        supports_correlation_logging=False,
    )


def config_of(case: Case) -> GenAiConfig:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        case["model"], PROVIDERS[case["provider"]]
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {case['model']}")
    return selected


def params_of(case: Case) -> GenericLiteLLMParams:
    params = case["litellm_params"]
    return GenericLiteLLMParams(
        api_key=params.get("api_key"),
        api_base=params.get("api_base"),
        model_info=params.get("model_info"),
    )


def invoke(
    call: GenAiCall[Answer],
    case: Case,
    config: GenAiConfig,
    logging_obj: Logging,
    client: HTTPHandler | AsyncHTTPHandler,
    is_async: bool,
) -> Answer:
    return call(
        model=case["model"],
        contents=case["contents"],
        generate_content_provider_config=config,
        generate_content_config_dict=case["config"],
        tools=case["tools"],
        custom_llm_provider=case["provider"],
        litellm_params=params_of(case),
        logging_obj=logging_obj,
        extra_headers=EXTRA.validate_python(case["extra_headers"]),
        extra_body=EXTRA.validate_python(case["extra_body"]),
        timeout=case["timeout"],
        _is_async=is_async,
        client=client,
        stream=case["stream"],
        system_instruction=case["system_instruction"],
    )


def details_of(logging_obj: Logging) -> dict[str, JsonValue]:
    details: dict[str, JsonValue] = logging_obj.model_call_details
    return details


def pre_call_view(logging_obj: Logging) -> dict[str, JsonValue]:
    details = details_of(logging_obj)
    args = DETAILS.validate_python(details["additional_args"])
    return {
        "input": details["input"],
        "api_base": args["api_base"],
        "headers": args["headers"],
        "complete_input_dict": args["complete_input_dict"],
    }


def http_view(logging_obj: Logging) -> dict[str, JsonValue] | None:
    details = details_of(logging_obj)
    if "httpx_response" not in details:
        return None
    reply = HttpReply.model_validate(details["httpx_response"])
    return {"status": reply.status_code, "text": reply.text}


def hidden_of(answer: SyncEvents | AsyncEvents) -> Hidden:
    return StreamState.model_validate(answer).hidden


def sync_events(answer: SyncEvents) -> list[JsonValue]:
    events: list[JsonValue] = []
    for chunk in answer:
        events.append(base64.b64encode(chunk).decode("ascii"))
    return events


async def async_events(answer: AsyncEvents) -> list[JsonValue]:
    events: list[JsonValue] = []
    async for chunk in answer:
        events.append(base64.b64encode(chunk).decode("ascii"))
    return events


def unary_report(
    recorder: Recorder,
    pre_call: dict[str, JsonValue],
    http: dict[str, JsonValue] | None,
    answer: Generated,
) -> dict[str, JsonValue]:
    return {
        "request": recorder.view(),
        "pre_call": pre_call,
        "response": DETAILS.validate_json(answer.model_dump_json()),
        "httpx_response": http,
        "hidden_params": None,
        "events": [],
    }


def stream_report(
    recorder: Recorder,
    pre_call: dict[str, JsonValue],
    http: dict[str, JsonValue] | None,
    hidden: Hidden,
    events: list[JsonValue],
) -> dict[str, JsonValue]:
    return {
        "request": recorder.view(),
        "pre_call": pre_call,
        "response": None,
        "httpx_response": http,
        "hidden_params": JSON.validate_python(hidden),
        "events": events,
    }


def sync_call(case: Case) -> JsonValue:
    recorder = Recorder(case)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
    logging_obj = logging_object(case)
    config = config_of(case)
    if case["stream"]:
        answer = invoke(SYNC_STREAM, case, config, logging_obj, client, False)
        hidden = hidden_of(answer)
        pre_call = pre_call_view(logging_obj)
        http = http_view(logging_obj)
        events = sync_events(answer)
        report = stream_report(recorder, pre_call, http, hidden, events)
        return JSON.validate_python(report)
    generated = invoke(SYNC_UNARY, case, config, logging_obj, client, False)
    pre_call = pre_call_view(logging_obj)
    http = http_view(logging_obj)
    return JSON.validate_python(unary_report(recorder, pre_call, http, generated))


async def async_call(case: Case) -> JsonValue:
    recorder = Recorder(case)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(recorder.handle))
    logging_obj = logging_object(case)
    config = config_of(case)
    if case["stream"]:
        answer = await invoke(ASYNC_STREAM, case, config, logging_obj, client, True)
        hidden = hidden_of(answer)
        pre_call = pre_call_view(logging_obj)
        http = http_view(logging_obj)
        events = await async_events(answer)
        report = stream_report(recorder, pre_call, http, hidden, events)
        return JSON.validate_python(report)
    generated = await invoke(ASYNC_UNARY, case, config, logging_obj, client, True)
    pre_call = pre_call_view(logging_obj)
    http = http_view(logging_obj)
    return JSON.validate_python(unary_report(recorder, pre_call, http, generated))


HANDLER: Final = BaseLLMHTTPHandler()
SYNC_UNARY: GenAiCall[Generated] = HANDLER.generate_content_handler
SYNC_STREAM: GenAiCall[SyncEvents] = HANDLER.generate_content_handler
ASYNC_UNARY: GenAiCall[Coroutine[JsonValue, JsonValue, Generated]] = (
    HANDLER.generate_content_handler
)
ASYNC_STREAM: GenAiCall[Coroutine[JsonValue, JsonValue, AsyncEvents]] = (
    HANDLER.generate_content_handler
)


def run(input_value: Case) -> JsonValue:
    if input_value["mode"] == "sync":
        return sync_call(input_value)
    return asyncio.run(async_call(input_value))

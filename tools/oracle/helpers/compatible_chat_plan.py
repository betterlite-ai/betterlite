import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.openai_like.chat.handler import OpenAILikeChatHandler
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode: TypeAlias = Literal["sync", "async"]
Reply: TypeAlias = ModelResponse | CustomStreamWrapper
Deferred: TypeAlias = Coroutine[None, None, Reply]
JSON: Final = TypeAdapter(JsonValue)
DEFAULT_TIMEOUT: Final = httpx.Timeout(600.0, connect=5.0)


class Call(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    custom_llm_provider: str
    api_base: str
    api_key: str | None
    headers: dict[str, str] | None
    optional_params: dict[str, JsonValue]
    custom_endpoint: bool | None
    timeout: float | None
    mode: Mode
    fake_stream: bool
    status: int
    reply: JsonValue
    watch_headers: list[str]


class Input(TypedDict):
    calls: list[Call]


class CompletionParams(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str
    custom_llm_provider: str
    custom_prompt_dict: dict[str, JsonValue]
    model_response: ModelResponse
    print_verbose: Callable[[str], None]
    encoding: None
    api_key: str | None
    logging_obj: Logging
    optional_params: dict[str, JsonValue]
    acompletion: bool
    litellm_params: dict[str, JsonValue]
    logger_fn: None
    headers: dict[str, str] | None
    timeout: float | None
    client: AsyncHTTPHandler | HTTPHandler
    custom_endpoint: bool | None
    streaming_decoder: None
    fake_stream: bool


# completion is unannotated, so this view names its parameters and its Any-laden return
class Completion(Protocol):
    def completion(
        self,
        *,
        model: str,
        messages: list[dict[str, JsonValue]],
        api_base: str,
        custom_llm_provider: str,
        custom_prompt_dict: dict[str, JsonValue],
        model_response: ModelResponse,
        print_verbose: Callable[[str], None],
        encoding: None,
        api_key: str | None,
        logging_obj: Logging,
        optional_params: dict[str, JsonValue],
        acompletion: bool,
        litellm_params: dict[str, JsonValue],
        logger_fn: None,
        headers: dict[str, str] | None,
        timeout: float | None,
        client: AsyncHTTPHandler | HTTPHandler,
        custom_endpoint: bool | None,
        streaming_decoder: None,
        fake_stream: bool,
    ) -> Reply | Deferred: ...


class ChatHandler(OpenAILikeChatHandler):
    # LiteLLM's __init__ chain is untyped and reaches only object's, so object's is the same call
    def __init__(self) -> None:
        object.__init__(self)


class ReplyFacts(BaseModel):
    model_config = ConfigDict(from_attributes=True, extra="ignore")
    model: str | None = None


class StreamFacts(BaseModel):
    model_config = ConfigDict(from_attributes=True, extra="ignore")
    model: str
    custom_llm_provider: str | None = None


class HiddenFields(BaseModel):
    model_config = ConfigDict(extra="ignore")
    model: str | None = None


class HiddenModel(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    hidden: HiddenFields = Field(alias="_hidden_params")


class TimeoutValues(BaseModel):
    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class Extensions(BaseModel):
    model_config = ConfigDict(extra="ignore")
    timeout: TimeoutValues | None = None


class RequestFacts(BaseModel):
    model_config = ConfigDict(from_attributes=True, extra="ignore")
    extensions: Extensions


class Transport:
    def __init__(self, status: int, reply: JsonValue) -> None:
        self.status = status
        self.reply = reply
        self.request: httpx.Request | None = None
        self.response: httpx.Response | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.request = request
        response = httpx.Response(
            self.status,
            headers={"Content-Type": "application/json"},
            stream=httpx.ByteStream(JSON.dump_json(self.reply)),
            request=request,
        )
        self.response = response
        return response


def quiet(_message: str) -> None:
    ...


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="compatible-chat-plan",
        function_id="fixture",
    )


def fixture_client(call: Call, transport: Transport) -> AsyncHTTPHandler | HTTPHandler:
    mock = httpx.MockTransport(transport.handle)
    if call["mode"] == "async":
        return AsyncHTTPHandler(transport=mock)
    timeout = call["timeout"]
    default = DEFAULT_TIMEOUT if timeout is None else httpx.Timeout(timeout)
    return HTTPHandler(client=httpx.Client(transport=mock, timeout=default, trust_env=False))


def completion_params(call: Call, transport: Transport) -> CompletionParams:
    return CompletionParams(
        model=call["model"],
        messages=call["messages"],
        api_base=call["api_base"],
        custom_llm_provider=call["custom_llm_provider"],
        custom_prompt_dict={},
        model_response=ModelResponse(),
        print_verbose=quiet,
        encoding=None,
        api_key=call["api_key"],
        logging_obj=logging_object(call["model"]),
        optional_params=dict(call["optional_params"]),
        acompletion=call["mode"] == "async",
        litellm_params={},
        logger_fn=None,
        headers=call["headers"],
        timeout=call["timeout"],
        client=fixture_client(call, transport),
        custom_endpoint=call["custom_endpoint"],
        streaming_decoder=None,
        fake_stream=call["fake_stream"],
    )


def watched_headers(request: httpx.Request, watch: list[str]) -> dict[str, JsonValue]:
    names = {name.lower() for name in watch}
    return {
        name.lower(): value for name, value in request.headers.items() if name.lower() in names
    }


def request_view(transport: Transport, watch: list[str]) -> JsonValue:
    request = transport.request
    if request is None:
        return None
    values = RequestFacts.model_validate(request).extensions.timeout
    timeout: JsonValue = None
    if values is not None and None not in (values.connect, values.read, values.write, values.pool):
        timeout = {
            "connect": values.connect,
            "read": values.read,
            "write": values.write,
            "pool": values.pool,
        }
    return {
        "url": str(request.url),
        "headers": watched_headers(request, watch),
        "body": JSON.validate_json(request.content),
        "timeout": timeout,
    }


def streamed(transport: Transport) -> bool | None:
    response = transport.response
    if response is None:
        return None
    return not response.is_stream_consumed


def settled(value: Reply | Deferred) -> Reply:
    if asyncio.iscoroutine(value):
        return asyncio.run(value)
    return value


def outcome_view(result: Reply, call: Call) -> dict[str, JsonValue]:
    if call["optional_params"].get("stream") is True:
        facts = StreamFacts.model_validate(result)
        return {
            "outcome": "stream",
            "reply": None,
            "stream": {"model": facts.model, "provider": facts.custom_llm_provider},
            "error": None,
        }
    reply = ReplyFacts.model_validate(result)
    hidden = HiddenModel.model_validate(result)
    return {
        "outcome": "reply",
        "reply": {"model": reply.model, "hidden_model": hidden.hidden.model},
        "stream": None,
        "error": None,
    }


def answer(handler: Completion, call: Call) -> JsonValue:
    transport = Transport(call["status"], call["reply"])
    params = completion_params(call, transport)
    view = outcome_view(settled(handler.completion(**params)), call)
    view["request"] = request_view(transport, call["watch_headers"])
    view["transport_stream"] = streamed(transport)
    return view


def run(input_value: Input) -> JsonValue:
    handler: Completion = ChatHandler()
    answers = [answer(handler, call) for call in input_value["calls"]]
    return {"calls": answers}

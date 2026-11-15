import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.openai_like.chat.handler import OpenAILikeChatHandler
from litellm.llms.openai_like.common_utils import OpenAILikeError
from litellm.types.utils import ModelResponse
from litellm.utils import CustomStreamWrapper
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode: TypeAlias = Literal["sync", "async"]
Reply: TypeAlias = ModelResponse | CustomStreamWrapper
Deferred: TypeAlias = Coroutine[None, None, Reply]
Complete: TypeAlias = Callable[..., Reply | Deferred]
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


# completion is unannotated on the handler, so its result is typed through this member
class Handler(Protocol):
    @property
    def completion(self) -> Complete: ...


class FailureFacts(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


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


def timeout_view(request: httpx.Request) -> JsonValue:
    values = RequestFacts.model_validate(request).extensions.timeout
    if values is None or None in (values.connect, values.read, values.write, values.pool):
        return None
    return {
        "connect": values.connect,
        "read": values.read,
        "write": values.write,
        "pool": values.pool,
    }


def watched_headers(request: httpx.Request, watch: list[str]) -> dict[str, JsonValue]:
    names = {name.lower() for name in watch}
    return {
        name.lower(): value for name, value in request.headers.items() if name.lower() in names
    }


def request_view(transport: Transport, watch: list[str]) -> JsonValue:
    request = transport.request
    if request is None:
        return None
    return {
        "url": str(request.url),
        "headers": watched_headers(request, watch),
        "body": JSON.validate_json(request.content),
        "timeout": timeout_view(request),
    }


def streamed(transport: Transport) -> bool | None:
    response = transport.response
    if response is None:
        return None
    return not response.is_stream_consumed


def settled(value: Reply | Deferred) -> Reply:
    match value:
        case ModelResponse() | CustomStreamWrapper():
            return value
        case Coroutine():
            return asyncio.run(value)


def outcome_view(result: Reply) -> dict[str, JsonValue]:
    match result:
        case ModelResponse():
            return {
                "outcome": "reply",
                "reply": {
                    "model": ReplyFacts.model_validate(result).model,
                    "hidden_model": HiddenModel.model_validate(result).hidden.model,
                },
                "stream": None,
                "error": None,
            }
        case CustomStreamWrapper():
            facts = StreamFacts.model_validate(result)
            return {
                "outcome": "stream",
                "reply": None,
                "stream": {"model": facts.model, "provider": facts.custom_llm_provider},
                "error": None,
            }


def failure_view(error: OpenAILikeError, transport: Transport, watch: list[str]) -> JsonValue:
    return {
        "request": request_view(transport, watch),
        "transport_stream": streamed(transport),
        "outcome": "error",
        "reply": None,
        "stream": None,
        "error": {
            "status": FailureFacts.model_validate(error).status_code,
            "message": str(error),
        },
    }


def answer(call: Call) -> JsonValue:
    transport = Transport(call["status"], call["reply"])
    client = fixture_client(call, transport)
    handler: Handler = OpenAILikeChatHandler()
    try:
        value = handler.completion(
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
            client=client,
            custom_endpoint=call["custom_endpoint"],
            streaming_decoder=None,
            fake_stream=call["fake_stream"],
        )
        result = settled(value)
    except OpenAILikeError as error:
        return failure_view(error, transport, call["watch_headers"])
    return {
        "request": request_view(transport, call["watch_headers"]),
        "transport_stream": streamed(transport),
        **outcome_view(result),
    }


def run(input_value: Input) -> JsonValue:
    answers = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}

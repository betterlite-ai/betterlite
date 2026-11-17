import asyncio
import copy
from collections.abc import Awaitable, Callable
from typing import Final, Literal, Protocol, TypedDict, Unpack, runtime_checkable
from unittest import mock

import httpx
import litellm
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.anthropic.chat.handler import AnthropicChatCompletion, ModelResponseIterator
from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from litellm.llms.anthropic.common_utils import AnthropicError, process_anthropic_headers
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

TIMEOUT: Final[float] = 1.5
SUCCESS: Final[int] = 200
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
JsonObject = dict[str, JsonValue]

Scenario = Literal[
    "messages_post_request",
    "buffered_success",
    "stream_request",
    "pre_stream_error",
    "native_error_body",
    "unsupported_embedding",
    "chat_failure_projection",
]


class Input(TypedDict):
    scenario: Scenario
    api_base: str
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    caller_headers: dict[str, str]
    api_key: str | None
    acompletion: bool
    transform_body: JsonObject
    transform_failure: str | None
    beta_matrix: JsonObject
    upstream_status: int | None
    upstream_headers: dict[str, str]
    upstream_body: str | None
    upstream_lines: list[str]
    failure_message: str | None
    request_header_names: list[str]
    response_header_names: list[str]


class TransformRecord(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]


class LoggedCall(TypedDict):
    input: JsonValue
    original_response: JsonValue
    body: JsonValue


class Upstream:
    def __init__(self, input_value: Input) -> None:
        self.status = input_value["upstream_status"]
        self.message = input_value["failure_message"]
        self.headers = input_value["upstream_headers"]
        self.body = (input_value["upstream_body"] or "").encode()
        self.requests: list[httpx.Request] = []
        self.responses: list[httpx.Response] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if self.status is None and self.message is not None:
            raise RuntimeError(self.message)
        response = httpx.Response(
            SUCCESS if self.status is None else self.status,
            headers=self.headers,
            stream=httpx.ByteStream(self.body),
        )
        self.responses.append(response)
        return response


class TransformArguments(TypedDict):
    model: str
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    headers: dict[str, str]


class ResponseArguments(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: "Recorder"
    request_data: JsonObject
    messages: list[JsonObject]
    optional_params: JsonObject
    litellm_params: JsonObject
    encoding: None
    api_key: str | None
    json_mode: bool | None


class PreCallArguments(TypedDict):
    input: JsonValue
    api_key: str | None
    additional_args: JsonObject


class PostCallArguments(TypedDict):
    input: JsonValue
    api_key: str | None
    original_response: object
    additional_args: JsonObject


# stands in for LiteLLM's Logging object, keeping only what the cases compare
class Recorder:
    def __init__(self, litellm_params: JsonObject) -> None:
        self.model_call_details: JsonObject = {"litellm_params": litellm_params}
        self.pre_input: JsonValue = None
        self.pre_body: JsonValue = None
        self.pre_api_base: JsonValue = None
        self.post_calls: list[LoggedCall] = []

    def pre_call(self, **kwargs: Unpack[PreCallArguments]) -> None:
        arguments = kwargs["additional_args"]
        # the body is kept by reference: the stream branches set data["stream"] after pre_call
        self.pre_input = kwargs["input"]
        self.pre_body = arguments.get("complete_input_dict")
        self.pre_api_base = arguments.get("api_base")

    def post_call(self, **kwargs: Unpack[PostCallArguments]) -> None:
        response = kwargs["original_response"]
        self.post_calls.append(
            LoggedCall(
                input=kwargs["input"],
                original_response=response if isinstance(response, str) else None,
                body=_json_text(kwargs["additional_args"].get("complete_input_dict")),
            )
        )


class FixtureConfig:
    def __init__(self, input_value: Input) -> None:
        self.transform_body = input_value["transform_body"]
        self.transform_failure = input_value["transform_failure"]
        self.transforms: list[TransformRecord] = []

    def transform_request(self, **kwargs: Unpack[TransformArguments]) -> JsonObject:
        return self._record(kwargs)

    async def async_transform_request(self, **kwargs: Unpack[TransformArguments]) -> JsonObject:
        await asyncio.sleep(0)
        return self._record(kwargs)

    def transform_response(self, **kwargs: Unpack[ResponseArguments]) -> ModelResponse:
        return MAPPER.transform_response(**kwargs)

    def _record(self, arguments: TransformArguments) -> JsonObject:
        self.transforms.append(
            TransformRecord(
                model=arguments["model"],
                messages=arguments["messages"],
                optional_params=arguments["optional_params"],
                litellm_params=arguments["litellm_params"],
                headers=arguments["headers"],
            )
        )
        failure = self.transform_failure
        if failure is not None:
            raise ValueError(failure)
        return copy.deepcopy(self.transform_body)


class MessageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    content: str | None


class ChoiceView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    message: MessageView
    finish_reason: str | None


class ReplyView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model: str | None
    choices: list[ChoiceView]


class OptionsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    json_mode: bool | None
    speed: str | None
    tool_name_reverse_map: dict[str, str] | None


class WrapperView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    model: str | None
    custom_llm_provider: str | None
    completion_stream: ModelResponseIterator


@runtime_checkable
class StatusCarrier(Protocol):
    status_code: int


@runtime_checkable
class Mapper(Protocol):
    def transform_response(self, **kwargs: Unpack[ResponseArguments]) -> ModelResponse: ...


ChatResult = ModelResponse | CustomStreamWrapper


class CompletionArguments(TypedDict):
    model: str
    messages: list[JsonObject]
    api_base: str
    custom_llm_provider: str
    custom_prompt_dict: JsonObject
    model_response: ModelResponse
    print_verbose: Callable[[str], None]
    encoding: None
    api_key: str | None
    logging_obj: Recorder
    optional_params: JsonObject
    timeout: float
    litellm_params: JsonObject
    acompletion: bool
    logger_fn: None
    headers: dict[str, str]
    client: AsyncHTTPHandler | HTTPHandler


@runtime_checkable
class AsyncChatHandler(Protocol):
    def completion(self, **kwargs: Unpack[CompletionArguments]) -> Awaitable[ChatResult]: ...


@runtime_checkable
class SyncChatHandler(Protocol):
    def completion(self, **kwargs: Unpack[CompletionArguments]) -> ChatResult: ...


@runtime_checkable
class EmbeddingHandler(Protocol):
    def embedding(self) -> None: ...


CALLS: Final = ConfigDict(arbitrary_types_allowed=True)
ASYNC_HANDLER: Final[AsyncChatHandler] = TypeAdapter(
    AsyncChatHandler, config=CALLS
).validate_python(AnthropicChatCompletion())
SYNC_HANDLER: Final[SyncChatHandler] = TypeAdapter(
    SyncChatHandler, config=CALLS
).validate_python(AnthropicChatCompletion())
MAPPER: Final[Mapper] = TypeAdapter(Mapper, config=CALLS).validate_python(AnthropicConfig())
EMBEDDING: Final[EmbeddingHandler] = TypeAdapter(
    EmbeddingHandler, config=CALLS
).validate_python(AnthropicChatCompletion())
PROCESS: Final[Callable[[httpx.Headers], dict[str, str]]] = TypeAdapter(
    Callable[[httpx.Headers], dict[str, str]]
).validate_python(process_anthropic_headers)


class Attempt:
    def __init__(self, input_value: Input, *, asynchronous: bool) -> None:
        self.config = FixtureConfig(input_value)
        self.recorder = Recorder(input_value["litellm_params"])
        self.upstream = Upstream(input_value)
        transport = httpx.MockTransport(self.upstream)
        client: AsyncHTTPHandler | HTTPHandler
        if asynchronous:
            client = AsyncHTTPHandler(transport=transport)
        else:
            client = HTTPHandler(client=httpx.Client(transport=transport))
        self.requests = self.upstream.requests
        self.failure: BaseException | None = None
        self.result: ChatResult | None = None
        arguments = CompletionArguments(
            model=input_value["model"],
            messages=input_value["messages"],
            api_base=input_value["api_base"],
            custom_llm_provider="anthropic",
            custom_prompt_dict={},
            model_response=ModelResponse(),
            print_verbose=_verbose,
            encoding=None,
            api_key=input_value["api_key"],
            logging_obj=self.recorder,
            optional_params=input_value["optional_params"],
            timeout=TIMEOUT,
            litellm_params=input_value["litellm_params"],
            acompletion=asynchronous,
            logger_fn=None,
            headers=dict(input_value["caller_headers"]),
            client=client,
        )
        with mock.patch.object(
            ProviderConfigManager, "get_provider_chat_config", return_value=self.config
        ):
            try:
                if asynchronous:
                    self.result = asyncio.run(_async_completion(arguments))
                else:
                    self.result = SYNC_HANDLER.completion(**arguments)
            except (ValueError, AnthropicError, litellm.AuthenticationError) as error:
                self.failure = error


async def _async_completion(arguments: CompletionArguments) -> ChatResult:
    return await ASYNC_HANDLER.completion(**arguments)


def _verbose(text: str) -> None:
    del text


def _json_text(value: JsonValue) -> JsonValue:
    if not isinstance(value, str):
        return value
    try:
        return JSON.validate_json(value)
    except ValidationError:
        return None


def _request_body(content: bytes) -> JsonValue:
    try:
        return JSON.validate_json(content)
    except ValidationError:
        return None


def _selected(headers: dict[str, str], names: list[str]) -> JsonValue:
    return JSON.validate_python({name: headers.get(name) for name in names})


def _request_value(request: httpx.Request, names: list[str]) -> JsonValue:
    return JSON.validate_python(
        {
            "url": str(request.url),
            "body": _request_body(request.content),
            "headers": _selected(dict(request.headers), names),
        }
    )


def _transform_value(record: TransformRecord, names: list[str]) -> JsonValue:
    return JSON.validate_python(
        {
            "model": record["model"],
            "messages": record["messages"],
            "optional_params": record["optional_params"],
            "litellm_params": record["litellm_params"],
            "headers": _selected(record["headers"], names),
        }
    )


def _reply_value(response: ModelResponse) -> JsonValue:
    view = ReplyView.model_validate(response)
    choice = view.choices[0] if view.choices else None
    return JSON.validate_python(
        {
            "model": view.model,
            "content": None if choice is None else choice.message.content,
            "finish_reason": None if choice is None else choice.finish_reason,
        }
    )


def _options(iterator: ModelResponseIterator) -> JsonValue:
    view = OptionsView.model_validate(iterator)
    return JSON.validate_python(
        {
            "json_mode": view.json_mode,
            "speed": view.speed,
            "tool_name_reverse_map": view.tool_name_reverse_map or {},
        }
    )


def _stream_value(wrapper: WrapperView, attempt: Attempt, names: list[str]) -> JsonValue:
    response = attempt.upstream.responses[0]
    return JSON.validate_python(
        {
            "model": wrapper.model,
            "provider": wrapper.custom_llm_provider,
            "options": _options(wrapper.completion_stream),
            "response_headers": _selected(PROCESS(response.headers), names),
        }
    )


def _pre_call_value(recorder: Recorder) -> JsonValue:
    return JSON.validate_python(
        {
            "input": recorder.pre_input,
            "body": recorder.pre_body,
            "api_base": recorder.pre_api_base,
        }
    )


def _call_value(call: LoggedCall) -> JsonValue:
    return JSON.validate_python(
        {
            "input": call["input"],
            "original_response": call["original_response"],
            "body": call["body"],
        }
    )


def _post_call_value(recorder: Recorder) -> JsonValue:
    if not recorder.post_calls:
        return JSON.validate_python({"input": None, "original_response": None, "body": None})
    return _call_value(recorder.post_calls[0])


def _failure_post_call(recorder: Recorder) -> JsonValue:
    if not recorder.post_calls:
        return None
    call = recorder.post_calls[0]
    return JSON.validate_python({"input": call["input"], "body": call["body"]})


def _status(attempt: Attempt) -> int | None:
    failure = attempt.failure
    if failure is None:
        return None
    if isinstance(failure, StatusCarrier):
        return failure.status_code
    return None


def _request_bundle(input_value: Input, *, asynchronous: bool) -> JsonValue:
    attempt = Attempt(input_value, asynchronous=asynchronous)
    transforms = attempt.config.transforms
    requests = attempt.requests
    return JSON.validate_python(
        {
            "transform": None
            if not transforms
            else _transform_value(transforms[0], input_value["request_header_names"]),
            "pre_call": _pre_call_value(attempt.recorder),
            "request": None
            if not requests
            else _request_value(requests[0], input_value["request_header_names"]),
        }
    )


def _reply_bundle(input_value: Input, *, asynchronous: bool) -> JsonValue:
    attempt = Attempt(input_value, asynchronous=asynchronous)
    if not isinstance(attempt.result, ModelResponse):
        raise ValueError("the buffered call did not return a reply")
    return JSON.validate_python(
        {
            "reply": _reply_value(attempt.result),
            "post_call": _post_call_value(attempt.recorder),
        }
    )


def _stream_bundle(input_value: Input, *, asynchronous: bool) -> JsonValue:
    attempt = Attempt(input_value, asynchronous=asynchronous)
    if not isinstance(attempt.result, CustomStreamWrapper):
        raise ValueError("the stream call did not open a stream")
    wrapper = WrapperView.model_validate(attempt.result)
    requests = attempt.requests
    return JSON.validate_python(
        {
            "stream": _stream_value(wrapper, attempt, input_value["response_header_names"]),
            "pre_call": _pre_call_value(attempt.recorder),
            "request": None
            if not requests
            else _request_value(requests[0], input_value["request_header_names"]),
            "post_call": None
            if not attempt.recorder.post_calls
            else _call_value(attempt.recorder.post_calls[0]),
        }
    )


def _failure_bundle(attempt: Attempt) -> JsonValue:
    return JSON.validate_python(
        {
            "status": _status(attempt),
            "posted": len(attempt.requests),
            "post_call": _failure_post_call(attempt.recorder),
        }
    )


def _messages_post_request(input_value: Input) -> JsonValue:
    return JSON.validate_python(
        {
            "async": _request_bundle(input_value, asynchronous=True),
            "sync": _request_bundle(input_value, asynchronous=False),
        }
    )


def _buffered_success(input_value: Input) -> JsonValue:
    return JSON.validate_python(
        {
            "async": _reply_bundle(input_value, asynchronous=True),
            "sync": _reply_bundle(input_value, asynchronous=False),
        }
    )


def _stream_request(input_value: Input) -> JsonValue:
    return JSON.validate_python(
        {
            "async": _stream_bundle(input_value, asynchronous=True),
            "sync": _stream_bundle(input_value, asynchronous=False),
        }
    )


def _pre_stream_error(input_value: Input) -> JsonValue:
    attempt = Attempt(input_value, asynchronous=input_value["acompletion"])
    return JSON.validate_python(
        {
            "status": _status(attempt),
            "posted": len(attempt.requests),
            "transformed": bool(attempt.config.transforms),
        }
    )


def _native_error_body(input_value: Input) -> JsonValue:
    attempt = Attempt(input_value, asynchronous=input_value["acompletion"])
    return JSON.validate_python(
        {
            "status": _status(attempt),
            "posted": len(attempt.requests),
            "post_call": _post_call_value(attempt.recorder),
        }
    )


def _unsupported_embedding(input_value: Input) -> JsonValue:
    del input_value
    return JSON.validate_python({"reply": EMBEDDING.embedding(), "posted": 0})


def _chat_failure_projection(input_value: Input) -> JsonValue:
    asynchronous = Attempt(input_value, asynchronous=True)
    synchronous = Attempt(input_value, asynchronous=False)
    return JSON.validate_python(
        {
            "async": _failure_bundle(asynchronous),
            "sync": _failure_bundle(synchronous),
        }
    )


SCENARIOS: Final[dict[str, Callable[[Input], JsonValue]]] = {
    "messages_post_request": _messages_post_request,
    "buffered_success": _buffered_success,
    "stream_request": _stream_request,
    "pre_stream_error": _pre_stream_error,
    "native_error_body": _native_error_body,
    "unsupported_embedding": _unsupported_embedding,
    "chat_failure_projection": _chat_failure_projection,
}


def run(input_value: Input) -> JsonValue:
    return SCENARIOS[input_value["scenario"]](input_value)

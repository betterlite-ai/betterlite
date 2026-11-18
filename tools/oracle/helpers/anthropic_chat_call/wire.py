import asyncio
from collections.abc import Awaitable, Callable
from typing import Final, Literal, Protocol, TypedDict, Unpack, runtime_checkable

import httpx
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.anthropic.chat.handler import AnthropicChatCompletion
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

TIMEOUT: Final[float] = 1.5
SUCCESS: Final[int] = 200
JsonObject = dict[str, JsonValue]
ChatResult = ModelResponse | CustomStreamWrapper
HEADERS: Final[TypeAdapter[dict[str, str] | None]] = TypeAdapter(dict[str, str] | None)

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


class PreCallArguments(TypedDict):
    input: JsonValue
    api_key: str | None
    additional_args: JsonObject


class PostCallArguments(TypedDict):
    input: JsonValue
    api_key: str
    original_response: JsonValue
    additional_args: JsonObject


# stands in for LiteLLM's Logging object, keeping only the fields the cases compare
class Recorder:
    def __init__(self) -> None:
        self.pre_input: JsonValue = None
        self.pre_body: JsonValue = None
        self.pre_api_base: JsonValue = None
        self.pre_beta: JsonValue = None
        self.post_input: JsonValue = None
        self.post_response: JsonValue = None
        self.post_body: JsonValue = None
        self.model_call_details: JsonObject = {}

    def pre_call(self, **kwargs: Unpack[PreCallArguments]) -> None:
        self.pre_input = kwargs["input"]
        arguments = kwargs["additional_args"]
        self.pre_body = arguments.get("complete_input_dict")
        self.pre_api_base = arguments.get("api_base")
        headers = HEADERS.validate_python(arguments.get("headers"))
        self.pre_beta = None if headers is None else headers.get("anthropic-beta")

    def post_call(self, **kwargs: Unpack[PostCallArguments]) -> None:
        self.post_input = kwargs["input"]
        self.post_response = kwargs["original_response"]
        arguments = kwargs["additional_args"]
        self.post_body = arguments.get("complete_input_dict")


class Upstream:
    def __init__(self, input_value: Input) -> None:
        self.status = input_value["upstream_status"]
        self.message = input_value["failure_message"]
        self.headers = input_value["upstream_headers"]
        body = input_value["upstream_body"]
        self.body = b"" if body is None else body.encode()
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if self.status is None and self.message is not None:
            raise RuntimeError(self.message)
        return httpx.Response(
            SUCCESS if self.status is None else self.status,
            headers=self.headers,
            stream=httpx.ByteStream(self.body),
        )


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
    def embedding(self) -> JsonValue: ...


class SilentVerbose:
    def __call__(self, text: str) -> None:
        del text


CALLS: Final = ConfigDict(arbitrary_types_allowed=True)
ASYNC_HANDLER: Final[AsyncChatHandler] = TypeAdapter(
    AsyncChatHandler, config=CALLS
).validate_python(AnthropicChatCompletion())
SYNC_HANDLER: Final[SyncChatHandler] = TypeAdapter(
    SyncChatHandler, config=CALLS
).validate_python(AnthropicChatCompletion())
EMBEDDING_HANDLER: Final[EmbeddingHandler] = TypeAdapter(
    EmbeddingHandler, config=CALLS
).validate_python(AnthropicChatCompletion())


async def _async_completion(arguments: CompletionArguments) -> ChatResult:
    pending = ASYNC_HANDLER.completion(**arguments)
    return await pending


class Attempt:
    def __init__(self, input_value: Input, *, asynchronous: bool) -> None:
        self.recorder = Recorder()
        self.upstream = Upstream(input_value)
        transport = httpx.MockTransport(self.upstream)
        client: AsyncHTTPHandler | HTTPHandler
        if asynchronous:
            client = AsyncHTTPHandler(transport=transport)
        else:
            client = HTTPHandler(client=httpx.Client(transport=transport))
        self.requests = self.upstream.requests
        self.result: ChatResult | None = None
        arguments = CompletionArguments(
            model=input_value["model"],
            messages=input_value["messages"],
            api_base=input_value["api_base"],
            custom_llm_provider="anthropic",
            custom_prompt_dict={},
            model_response=ModelResponse(),
            print_verbose=SilentVerbose(),
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
        if asynchronous:
            self.result = asyncio.run(_async_completion(arguments))
        else:
            self.result = SYNC_HANDLER.completion(**arguments)


def result_of(attempt: Attempt) -> ChatResult:
    result = attempt.result
    if result is None:
        raise ValueError("the call returned no result")
    return result

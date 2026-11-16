from collections.abc import Awaitable, Callable
from typing import Literal, Protocol, TypedDict, Unpack, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.anthropic.chat import handler as anthropic_handler
from litellm.llms.anthropic.chat.handler import AnthropicChatCompletion, ModelResponseIterator
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Failure = Literal["unknown", "timeout"]
Opened = tuple[ModelResponseIterator, httpx.Headers]

class Input(TypedDict):
    scenario: str
    api_base: str
    messages: list[dict[str, JsonValue]]
    model: str
    json_mode: bool
    headers: dict[str, str]
    request_header_names: list[str]
    response_header_names: list[str]
    data: str
    request_data: dict[str, JsonValue]
    speed: str | None
    reverse_map: dict[str, str]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    upstream_status: int | None
    upstream_headers: dict[str, str]
    upstream_lines: list[str]
    upstream_body: str | None
    failure_message: str | None

class Upstream:
    def __init__(self, input_value: Input, status: int) -> None:
        self.status = status
        self.headers = input_value["upstream_headers"]
        lines = "\n".join(input_value["upstream_lines"])
        body = input_value["upstream_body"]
        self.body = lines.encode() if body is None else body.encode()
        self.requests: list[httpx.Request] = []
        self.responses: list[httpx.Response] = []

    def reply(self) -> httpx.Response:
        response = httpx.Response(
            self.status, headers=self.headers, stream=httpx.ByteStream(self.body)
        )
        self.responses.append(response)
        return response

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return self.reply()

class SyncUpstream(Upstream):
    def post(self, url: str, **options: JsonValue) -> httpx.Response:
        del url, options
        return self.reply()

class AsyncGuard(Upstream):
    async def post(self, url: str, **options: JsonValue) -> httpx.Response:
        del url, options
        return self.reply()

class Raiser:
    def __init__(self, failure: Failure, message: str | None) -> None:
        self.failure = failure
        self.message = "the transport exploded" if message is None else message

    def __call__(self, request: httpx.Request) -> httpx.Response:
        del request
        if self.failure == "timeout":
            raise httpx.ReadTimeout(self.message)
        raise RuntimeError(self.message)

Client = AsyncHTTPHandler | HTTPHandler | Upstream

class ErrorView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    status_code: int
    message: bytes
    headers: httpx.Headers | dict[str, str] | None = None

class OptionsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    json_mode: bool | None
    speed: str | None
    tool_name_reverse_map: dict[str, str]

class WrapperView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    model: str
    custom_llm_provider: str | None
    completion_stream: ModelResponseIterator

class CallArguments(TypedDict):
    client: Client | None
    api_base: str
    headers: dict[str, str]
    data: str
    model: str
    messages: list[dict[str, JsonValue]]
    logging_obj: Logging
    timeout: float
    json_mode: bool
    speed: str | None
    tool_name_reverse_map: dict[str, str] | None

@runtime_checkable
class Handlers(Protocol):
    def make_call(self, **kwargs: Unpack[CallArguments]) -> Awaitable[Opened]: ...

    def make_sync_call(self, **kwargs: Unpack[CallArguments]) -> Opened: ...

    def process_anthropic_headers(self, headers: httpx.Headers) -> dict[str, str]: ...

class StreamArguments(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str
    custom_prompt_dict: dict[str, JsonValue]
    model_response: ModelResponse
    print_verbose: Callable[[str], None] | None
    timeout: float
    client: AsyncHTTPHandler
    encoding: None
    api_key: str
    logging_obj: Logging
    stream: bool
    _is_function_call: bool
    data: dict[str, JsonValue]
    json_mode: bool
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]

@runtime_checkable
class CompletionHandlers(Protocol):
    def acompletion_stream_function(
        self, **kwargs: Unpack[StreamArguments]
    ) -> Awaitable[CustomStreamWrapper]: ...

CALLS: TypeAdapter[Handlers] = TypeAdapter(
    Handlers, config=ConfigDict(arbitrary_types_allowed=True)
)
HANDLER: Handlers = CALLS.validate_python(anthropic_handler)
COMPLETION: Callable[[], CompletionHandlers] = TypeAdapter(
    Callable[[], CompletionHandlers]
).validate_python(AnthropicChatCompletion)

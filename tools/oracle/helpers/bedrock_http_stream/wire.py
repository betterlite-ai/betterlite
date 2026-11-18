from collections.abc import AsyncIterator, Awaitable, Iterator
from typing import Final, Literal, Protocol, TypedDict, Unpack, runtime_checkable

import httpx
from litellm.llms.bedrock.chat import converse_handler, invoke_handler
from litellm.types.utils import ModelResponseStream
from pydantic import ConfigDict, JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BYTES: Final[TypeAdapter[bytes]] = TypeAdapter(bytes)
FakeFailure = Literal["timeout", "other"]
Consume = Literal["none", "first", "all"]
Consumed = tuple[list[JsonValue], list[JsonValue]]
Chunk = ModelResponseStream | JsonValue

class Compare(TypedDict):
    message: bool
    headers: bool

class Case(TypedDict):
    path: Literal["invoke_sync", "invoke_async", "converse_sync"]
    api_base: str
    headers: dict[str, str]
    data: str
    signed_json_body: str | None
    model: str
    messages: list[dict[str, JsonValue]]
    json_mode: bool | None
    fake_stream: bool
    status: int
    response_headers: dict[str, str]
    body: str
    failure: FakeFailure | None
    consume: Consume
    compare: Compare
    provider: str | None

class Input(TypedDict):
    cases: list[Case]

@runtime_checkable
class SyncChunks(Protocol):
    def __iter__(self) -> Iterator[Chunk]: ...

    def __next__(self) -> Chunk: ...

@runtime_checkable
class AsyncChunks(Protocol):
    def __aiter__(self) -> AsyncIterator[Chunk]: ...

    def __anext__(self) -> Awaitable[Chunk]: ...

OpenedSync = tuple[SyncChunks, httpx.Headers]
OpenedAsync = tuple[AsyncChunks, httpx.Headers]

class LogArguments(TypedDict):
    input: list[dict[str, JsonValue]]
    api_key: str
    original_response: str
    additional_args: dict[str, JsonValue]

class Recorder:
    def __init__(self) -> None:
        self.calls: list[dict[str, JsonValue]] = []

    def post_call(self, **params: Unpack[LogArguments]) -> None:
        args = params["additional_args"]
        self.calls.append({
            "input": JSON.validate_python(params["input"]),
            "original_response": params["original_response"],
            "complete_input_dict": args["complete_input_dict"],
        })

class PostOptions(TypedDict):
    headers: dict[str, str]
    data: str | bytes
    stream: bool
    logging_obj: Recorder

class Fixture:
    def __init__(self, case: Case) -> None:
        self.status = case["status"]
        self.headers = case["response_headers"]
        self.body = bytes.fromhex(case["body"])
        self.failure = case["failure"]
        self.posts: list[dict[str, JsonValue]] = []
        self.recording = Recorder()

    def reply(self, url: str, options: PostOptions) -> httpx.Response:
        pairs = sorted([name, value] for name, value in options["headers"].items())
        payload: list[int] = list(BYTES.validate_python(options["data"]))
        self.posts.append({
            "url": url,
            "body": JSON.validate_python(payload),
            "headers": JSON.validate_python(pairs),
            "stream": options["stream"],
        })
        if self.failure == "timeout":
            raise httpx.ReadTimeout("the request timed out")
        if self.failure == "other":
            raise httpx.ConnectError("the connection was refused")
        response = httpx.Response(
            self.status, headers=self.headers, stream=httpx.ByteStream(self.body)
        )
        if not options["stream"]:
            response.read()
        return response

    def result(
        self,
        headers: httpx.Headers | None,
        values: list[JsonValue],
        failures: list[JsonValue],
        error: JsonValue | None,
    ) -> JsonValue:
        response_headers: JsonValue = None
        if headers is not None:
            pairs = sorted([name, value] for name, value in headers.items())
            response_headers = JSON.validate_python(pairs)
        payload: dict[str, JsonValue] = {
            "post": None if not self.posts else self.posts[0],
            "error": error,
            "response_headers": response_headers,
            "chunks": values,
            "failures": failures,
            "logs": JSON.validate_python(self.recording.calls),
        }
        return JSON.validate_python(payload)

class SyncClient(Fixture):
    def post(self, url: str, **options: Unpack[PostOptions]) -> httpx.Response:
        return self.reply(url, options)

class AsyncClient(Fixture):
    async def post(self, url: str, **options: Unpack[PostOptions]) -> httpx.Response:
        return self.reply(url, options)

class OpenArguments(TypedDict):
    api_base: str
    headers: dict[str, str]
    data: str
    model: str
    messages: list[dict[str, JsonValue]]
    logging_obj: Recorder
    json_mode: bool | None
    fake_stream: bool

class ConverseArguments(TypedDict):
    client: SyncClient
    api_base: str
    headers: dict[str, str]
    data: str
    model: str
    messages: list[dict[str, JsonValue]]
    logging_obj: Recorder
    json_mode: bool | None
    fake_stream: bool

class InvokeSyncArguments(TypedDict):
    client: SyncClient
    api_base: str
    headers: dict[str, str]
    data: str
    signed_json_body: bytes | None
    model: str
    messages: list[dict[str, JsonValue]]
    logging_obj: Recorder
    fake_stream: bool
    json_mode: bool | None
    bedrock_invoke_provider: str | None

class InvokeAsyncArguments(TypedDict):
    client: AsyncClient
    api_base: str
    headers: dict[str, str]
    data: str
    model: str
    messages: list[dict[str, JsonValue]]
    logging_obj: Recorder
    fake_stream: bool
    json_mode: bool | None
    bedrock_invoke_provider: str | None

@runtime_checkable
class ConverseSync(Protocol):
    def make_sync_call(self, **kwargs: Unpack[ConverseArguments]) -> OpenedSync: ...

@runtime_checkable
class InvokeSync(Protocol):
    def make_sync_call(self, **kwargs: Unpack[InvokeSyncArguments]) -> OpenedSync: ...

@runtime_checkable
class InvokeAsync(Protocol):
    def make_call(self, **kwargs: Unpack[InvokeAsyncArguments]) -> Awaitable[OpenedAsync]: ...

CONVERSE: ConverseSync = TypeAdapter(
    ConverseSync, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(converse_handler)
INVOKE_SYNC: InvokeSync = TypeAdapter(
    InvokeSync, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(invoke_handler)
INVOKE_ASYNC: InvokeAsync = TypeAdapter(
    InvokeAsync, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(invoke_handler)

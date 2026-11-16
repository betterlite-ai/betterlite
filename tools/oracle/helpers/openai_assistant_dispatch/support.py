import ssl as ssl_module
from collections.abc import Coroutine, Iterable
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack

import httpx
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import Thread
from litellm.types.realtime import RealtimeQueryParams
from openai.lib.streaming import (
    AssistantEventHandler,
    AsyncAssistantEventHandler,
    AsyncAssistantStreamManager,
)
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta import Assistant, AssistantDeleted, AssistantToolParam
from openai.types.beta.thread_create_params import Message as NewThreadMessage
from openai.types.beta.threads import Message, Run
from pydantic import JsonValue, TypeAdapter

ORG = "org-fixture"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
SslValue: TypeAlias = bool | str | ssl_module.SSLContext | None


class Params(TypedDict):
    order: str | None
    limit: int | None
    before: str | None
    after: str | None


class Canned(TypedDict):
    status: int
    body: NotRequired[JsonValue]
    text: NotRequired[str]
    content_type: NotRequired[str]


class RunFields(TypedDict):
    thread_id: str
    assistant_id: str
    additional_instructions: str | None
    instructions: str | None
    metadata: dict[str, str] | None
    model: str | None
    tools: list[AssistantToolParam] | None


class Input(TypedDict):
    kind: Literal["assistants", "threads", "runs", "decisions", "realtime"]
    operation: str
    api_base: str | None
    api_key: str | None
    params: Params
    aget_assistants: bool | None
    arun_thread: bool
    data: dict[str, JsonValue]
    request: RunFields
    decision: dict[str, JsonValue]
    payload: str
    model: str
    replies: list[Canned]
    assistant_id: str
    thread_id: str
    query_params: RealtimeQueryParams | None
    scope_headers: list[tuple[str, str]]
    close_error: str | None
    handshake_status: int | None
    environment: dict[str, str]


# Each call lists the keywords of one LiteLLM method, so the protocols type its untyped dicts.
class Credentials(TypedDict):
    api_key: str | None
    api_base: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


class Client(Credentials):
    client: openai.AsyncOpenAI | None


class Listing(Client):
    order: str | None
    limit: int | None
    before: str | None
    after: str | None


class NewAssistant(Client):
    create_assistant_data: dict[str, JsonValue]


class AssistantRef(Client):
    assistant_id: str


class ThreadRef(Client):
    thread_id: str


class NewMessage(ThreadRef):
    message_data: dict[str, JsonValue]


class NewThread(Client):
    metadata: dict[str, str] | None
    messages: Iterable[NewThreadMessage] | None


class SyncListing(Credentials):
    client: openai.OpenAI | None
    aget_assistants: Literal[False]


class AsyncListing(Credentials):
    client: openai.AsyncOpenAI | None
    aget_assistants: Literal[True]


class SyncRun(RunFields, Credentials):
    stream: bool | None
    client: openai.OpenAI
    arun_thread: Literal[False]
    event_handler: AssistantEventHandler | None


class AsyncRun(RunFields, Credentials):
    stream: bool | None
    client: openai.AsyncOpenAI
    arun_thread: Literal[True]
    event_handler: AssistantEventHandler | None


class StreamRun(RunFields):
    client: openai.AsyncOpenAI
    event_handler: AssistantEventHandler | None


class ClientOptions(TypedDict):
    api_key: str | None
    base_url: str | None
    organization: str
    max_retries: int


class NewThreadBody(TypedDict):
    messages: list[NewThreadMessage] | None
    metadata: dict[str, str] | None


THREAD_BODY: TypeAdapter[NewThreadBody] = TypeAdapter(NewThreadBody)


class Helpers(Protocol):
    async def async_get_assistants(self, **call: Unpack[Listing]) -> AsyncCursorPage[Assistant]: ...

    async def async_create_assistants(self, **call: Unpack[NewAssistant]) -> Assistant: ...

    async def async_delete_assistant(self, **call: Unpack[AssistantRef]) -> AssistantDeleted: ...

    async def a_add_message(self, **call: Unpack[NewMessage]) -> Message: ...

    async def async_get_messages(self, **call: Unpack[ThreadRef]) -> AsyncCursorPage[Message]: ...

    async def async_create_thread(self, **call: Unpack[NewThread]) -> Thread: ...

    async def async_get_thread(self, **call: Unpack[ThreadRef]) -> Thread: ...

    def async_run_thread_stream(
        self, **call: Unpack[StreamRun]
    ) -> AsyncAssistantStreamManager[AsyncAssistantEventHandler]: ...


class SyncFacade(Protocol):
    def get_assistants(self, **call: Unpack[SyncListing]) -> SyncCursorPage[Assistant]: ...

    def run_thread(self, **call: Unpack[SyncRun]) -> Run: ...


class AsyncFacade(Protocol):
    def get_assistants(
        self, **call: Unpack[AsyncListing]
    ) -> Coroutine[None, None, AsyncCursorPage[Assistant]]: ...

    def run_thread(self, **call: Unpack[AsyncRun]) -> Coroutine[None, None, Run]: ...


class ClientSocket(Protocol):
    async def send_text(self, data: str) -> None: ...

    async def receive_text(self) -> str: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


# the handler only matches this protocol when the call names timeout and litellm_metadata too
class RealtimeCall(TypedDict):
    model: str
    websocket: ClientSocket
    logging_obj: Logging
    api_base: str | None
    api_key: str | None
    timeout: float | None
    query_params: RealtimeQueryParams | None
    litellm_metadata: dict[str, JsonValue] | None


class RealtimeRunner(Protocol):
    async def async_realtime(self, **call: Unpack[RealtimeCall]) -> None: ...


class Wire:
    def __init__(self, replies: list[Canned]) -> None:
        self.replies = list(replies)
        self.seen: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        content = request.content
        headers = request.headers
        row: dict[str, JsonValue] = {
            "method": request.method,
            "url": str(request.url),
            "authorization": first_header(headers.get_list("authorization")),
            "openai-organization": first_header(headers.get_list("openai-organization")),
            "openai-beta": first_header(headers.get_list("openai-beta")),
            "content-type": first_header(headers.get_list("content-type")),
            "body": JSON.validate_json(content) if content else None,
        }
        self.seen.append(row)
        canned = self.replies.pop(0)
        text = canned.get("text")
        answer = JSON.dump_json(canned.get("body")) if text is None else text.encode()
        content_type = canned.get("content_type")
        sent = {} if content_type is None else {"content-type": content_type}
        return httpx.Response(canned["status"], content=answer, headers=sent)


def first_header(values: list[str]) -> str | None:
    if values:
        return values[0]
    return None


def credentials(input_value: Input) -> Credentials:
    return {
        "api_key": input_value["api_key"],
        "api_base": input_value["api_base"],
        "timeout": 30.0,
        "max_retries": 0,
        "organization": ORG,
    }


def client_options(input_value: Input) -> ClientOptions:
    return {
        "api_key": input_value["api_key"],
        "base_url": input_value["api_base"],
        "organization": ORG,
        "max_retries": 0,
    }


def async_client(input_value: Input, wire: Wire) -> openai.AsyncOpenAI:
    transport = httpx.MockTransport(wire.handle)
    http_client = httpx.AsyncClient(transport=transport)
    return openai.AsyncOpenAI(**client_options(input_value), http_client=http_client)


def sync_client(input_value: Input, wire: Wire) -> openai.OpenAI:
    transport = httpx.MockTransport(wire.handle)
    http_client = httpx.Client(transport=transport)
    return openai.OpenAI(**client_options(input_value), http_client=http_client)

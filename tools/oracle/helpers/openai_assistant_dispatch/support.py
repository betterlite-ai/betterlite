import ssl as ssl_module
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack

import httpx
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.realtime import RealtimeQueryParams
from openai.types.beta import AssistantToolParam
from openai.types.beta.thread_create_params import Message as NewThreadMessage
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


# Each call lists one LiteLLM method's keywords, so a typed dict names the kwargs it unpacks.
class Credentials(TypedDict):
    api_key: str | None
    api_base: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


class ClientOptions(TypedDict):
    api_key: str | None
    base_url: str | None
    organization: str
    max_retries: int


class NewThreadBody(TypedDict):
    messages: list[NewThreadMessage] | None
    metadata: dict[str, str] | None


THREAD_BODY: TypeAdapter[NewThreadBody] = TypeAdapter(NewThreadBody)


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

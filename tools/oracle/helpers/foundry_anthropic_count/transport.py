import logging
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    runtime_checkable,
)

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import LlmProviders
from pydantic import ConfigDict, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

DECODER: TypeAdapter[Json] = TypeAdapter(Json)

Env: TypeAlias = dict[str, str]
Params: TypeAlias = dict[str, Json]
Messages: TypeAlias = list[dict[str, object]]


class Reply(TypedDict):
    status: NotRequired[int]
    body: str


class Globals(TypedDict):
    api_key: NotRequired[str | None]
    azure_key: NotRequired[str | None]


class HandleCase(TypedDict):
    model: str
    messages: Messages
    api_key: str
    api_base: str
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    timeout: NotRequired[float]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    reply: NotRequired[Reply]


class CounterCase(TypedDict):
    model: str
    messages: NotRequired[Messages | None]
    contents: NotRequired[Messages | None]
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    provider: NotRequired[str]
    request_model: NotRequired[str]
    reply: NotRequired[Reply]


class EndpointCall(TypedDict):
    function: Literal["endpoint"]
    api_bases: list[str]


class HandleCall(TypedDict):
    function: Literal["handle"]
    cases: list[HandleCase]


class CounterCall(TypedDict):
    function: Literal["counter"]
    cases: list[CounterCase]


Input: TypeAlias = EndpointCall | HandleCall | CounterCall


CLIENT_KEY: Final = "async_httpx_client"
CACHE_KEY: Final = CLIENT_KEY + LlmProviders.AZURE_AI
CACHE_TTL: Final = 600
HEADERS: Final = (
    "content-type",
    "x-api-key",
    "api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
)
SUCCESS: Final = 200
SECONDS: TypeAdapter[dict[str, float]] = TypeAdapter(dict[str, float])


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)


class Sent:
    def __init__(self, reply: Reply | None) -> None:
        self.reply = reply
        self.first: Json = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        if self.first is None:
            headers = {name.lower(): value for name, value in request.headers.items()}
            seconds = SECONDS.validate_python(request.extensions["timeout"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/anthropic/count_tokens/handler.py#L84-L93
            body: Json = None if not request.content else DECODER.validate_json(request.content)
            self.first = {
                "url": str(request.url),
                "timeout": seconds["read"],
                "headers": {name: headers.get(name) for name in HEADERS},
                "body": body,
            }
        if self.reply is None:
            raise RuntimeError("fixture transport failure")
        status = self.reply.get("status")
        return httpx.Response(
            SUCCESS if status is None else status,
            content=self.reply["body"].encode(),
            request=request,
        )


class Warnings(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        if record.levelno == logging.WARNING:
            self.entries.append(warning_entry(record.getMessage()))
        return True


def warning_entry(message: str) -> Json:
    subject, _separator, detail = message.partition(": ")
    status: Json = None
    if detail.startswith("status="):
        head = detail.removeprefix("status=").split(",", 1)[0]
        try:
            status = int(head)
        except ValueError:
            status = None
    return {"subject": subject, "status": status}


def install(sent: Sent) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    cache: ClientCache = CLIENT_CACHE.validate_python(holder)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(sent.handle))
    cache.set_cache(key=CACHE_KEY, value=client, ttl=CACHE_TTL)

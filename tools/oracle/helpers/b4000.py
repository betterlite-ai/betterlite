import asyncio
import io
import os
import re
import sys
from collections.abc import Mapping, Sequence
from datetime import timedelta
from types import ModuleType
from typing import ClassVar, Literal, NotRequired, Protocol, TypedDict

import azure.identity
import httpx
import litellm
import requests
from azure.core.pipeline.transport._requests_asyncio import AsyncioRequestsTransport
from azure.storage.filedatalake.aio import DataLakeServiceClient
from litellm.integrations.azure_storage.azure_storage import AzureBlobStorageLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, httpxSpecialProvider
from litellm.proxy import proxy_server
from pydantic import JsonValue, TypeAdapter
from requests.adapters import HTTPAdapter
from typing_extensions import override
from urllib3.response import HTTPResponse

AIO_MODULE = "azure.storage.filedatalake.aio"
IDENTITY_MODULE = azure.identity.__name__
START_SECONDS = 1_700_000_000.0
BEARER = "tok-1"
JSON_VALUE = TypeAdapter(JsonValue)
# LiteLLM's event methods need premium_user; a licensed setup has it, and betterlite never checks
proxy_server.premium_user = True


class Reply(TypedDict):
    status: int
    body: str


class Log(TypedDict):
    do: Literal["event", "upload"]
    id: str
    failure: NotRequired[bool]


class Flush(TypedDict):
    do: Literal["flush"]


class Advance(TypedDict):
    do: Literal["advance"]
    seconds: int


Action = Log | Flush | Advance


class Scenario(TypedDict):
    environment: dict[str, str]
    actions: list[Action]
    payload: NotRequired[dict[str, JsonValue]]
    replies: NotRequired[list[Reply]]


class Input(TypedDict):
    scenarios: list[Scenario]


class Queued(TypedDict):
    id: str


class Queue(Protocol):
    @property
    def log_queue(self) -> Sequence[Queued]: ...


class Flusher(Protocol):
    async def flush_queue(self) -> None: ...


class EventSink(Protocol):
    async def async_log_success_event(
        self, kwargs: dict[str, JsonValue], response_obj: None, start_time: float, end_time: float
    ) -> None: ...

    async def async_log_failure_event(
        self, kwargs: dict[str, JsonValue], response_obj: None, start_time: float, end_time: float
    ) -> None: ...


class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler) -> None: ...


class Wire:
    # LiteLLM builds its HTTP and SDK clients itself, so what they send lives on the class
    calls: ClassVar[list[JsonValue]] = []
    replies: ClassVar[list[Reply]] = []
    created: ClassVar[int] = 0
    closed: ClassVar[int] = 0


class ManualClock:
    def __init__(self) -> None:
        self.now = START_SECONDS

    def __call__(self) -> float:
        return self.now


# every scripted operation answers with the status its SDK call treats as success
def classify_call(method: str, url: str) -> tuple[str, int]:
    if method == "HEAD":
        return "exists", 200
    if "resource=directory" in url:
        return "create_directory", 201
    if "resource=file" in url:
        return "create_file", 201
    if "action=append" in url:
        return "append", 202
    if "action=flush" in url:
        return "flush", 200
    return "other", 200


def answer(method: str, url: str, body: bytes) -> tuple[int, str]:
    op, success_status = classify_call(method, url)
    # the flush position counts JSON bytes, and json.dumps spacing is outside the parity contract
    masked = re.sub(r"(\?action=flush&position=)\d+", r"\1{bytes}", url)
    entry: dict[str, JsonValue] = {"op": op, "url": re.sub(r"\d{4}-\d{2}-\d{2}", "{date}", masked)}
    if op == "append":
        text = body.decode("utf-8")
        entry["body"] = JSON_VALUE.validate_json(text.rstrip("\n"))
        entry["trailing_newline"] = text.endswith("\n")
    Wire.calls.append(entry)
    if Wire.replies:
        reply = Wire.replies.pop(0)
        return reply["status"], reply["body"]
    return success_status, ""


def serve(request: httpx.Request) -> httpx.Response:
    status, text = answer(request.method, str(request.url), request.content)
    return httpx.Response(status, text=text, request=request)


class ScriptedAdapter(HTTPAdapter):
    @override
    def send(
        self,
        request: requests.PreparedRequest,
        stream: bool = False,
        timeout: None | float | tuple[float, float] | tuple[float, None] = None,
        verify: bool | str = True,
        cert: None | bytes | str | tuple[bytes | str, bytes | str] = None,
        proxies: Mapping[str, str] | None = None,
    ) -> requests.Response:
        del stream, timeout, verify, cert, proxies
        raw = b""
        if request.body is not None:
            match request.body:
                case bytes() as content:
                    raw = content
                case str() as text:
                    raw = text.encode("utf-8")
        status, text = answer(f"{request.method}", f"{request.url}", raw)
        payload = text.encode("utf-8")
        reply = HTTPResponse(
            body=io.BytesIO(payload),
            headers={"Content-Length": str(len(payload))},
            status=status,
            preload_content=False,
        )
        return self.build_response(request, reply)


class ScriptedClient(DataLakeServiceClient):
    def __init__(self, account_url: str, credential: str) -> None:
        Wire.created += 1
        session = requests.Session()
        session.mount("https://", ScriptedAdapter())
        super().__init__(
            account_url=account_url,
            credential=credential,
            transport=AsyncioRequestsTransport(session=session),
        )

    @override
    async def close(self) -> None:
        Wire.closed += 1
        await super().close()


class ScriptedProvider:
    def __init__(self, _credential: "ScriptedCredential", _scope: str) -> None: ...

    def __call__(self) -> str:
        Wire.calls.append({"op": "entra_token"})
        return BEARER


class ScriptedCredential:
    def __init__(self, _tenant_id: str, _client_id: str, _client_secret: str) -> None: ...


class ScriptedAio(ModuleType):
    DataLakeServiceClient: type[ScriptedClient] = ScriptedClient


class ScriptedIdentity(ModuleType):
    ClientSecretCredential: type[ScriptedCredential] = ScriptedCredential
    get_bearer_token_provider: type[ScriptedProvider] = ScriptedProvider


class NoCredentialChain:
    def __call__(self) -> ScriptedProvider:
        raise RuntimeError("no credential in the chain")


async def perform(
    action: Action,
    logger: AzureBlobStorageLogger,
    sink: EventSink,
    clock: ManualClock,
    base: dict[str, JsonValue],
) -> JsonValue:
    try:
        if action["do"] == "event":
            kwargs: dict[str, JsonValue] = {"standard_logging_object": {**base, "id": action["id"]}}
            if action.get("failure"):
                await sink.async_log_failure_event(kwargs, None, 0.0, 0.0)
            else:
                await sink.async_log_success_event(kwargs, None, 0.0, 0.0)
        # the per-payload upload's payload type carries Any, so queue one event and flush
        if action["do"] == "upload":
            event: dict[str, JsonValue] = {"standard_logging_object": {**base, "id": action["id"]}}
            await sink.async_log_success_event(event, None, 0.0, 0.0)
        if action["do"] in {"upload", "flush"}:
            flusher: Flusher = logger
            await flusher.flush_queue()
    except (ValueError, RuntimeError, httpx.HTTPError):
        return {"raised": True}
    if action["do"] == "advance":
        clock.now += action["seconds"]
        if logger.token_expiry is not None:
            logger.token_expiry -= timedelta(seconds=action["seconds"])
    return None


def apply_environment(environment: dict[str, str]) -> None:
    for name in list(os.environ):
        if name.startswith("AZURE_STORAGE_"):
            del os.environ[name]
    for name, value in environment.items():
        os.environ[name] = value


async def drive(scenario: Scenario, cache: ClientCache) -> JsonValue:
    apply_environment(scenario["environment"])
    replies = scenario.get("replies")
    Wire.calls, Wire.replies = [], [] if replies is None else list(replies)
    Wire.created = Wire.closed = 0
    clock = ManualClock()
    try:
        logger = AzureBlobStorageLogger(
            build_credential_chain_token_provider=NoCredentialChain(), clock=clock
        )
    except ValueError as error:
        return {"initialized": False, "error": str(error)}
    cache.set_cache(
        key="async_httpx_client" + httpxSpecialProvider.LoggingCallback,
        value=AsyncHTTPHandler(transport=httpx.MockTransport(serve)),
    )
    payload = scenario.get("payload")
    base = {} if payload is None else payload
    sys.modules[AIO_MODULE] = ScriptedAio(AIO_MODULE)
    sys.modules[IDENTITY_MODULE] = ScriptedIdentity(IDENTITY_MODULE)
    results = [await perform(step, logger, logger, clock, base) for step in scenario["actions"]]
    queue: Queue = logger
    return {
        "initialized": True,
        "endpoints": {
            "dfs": logger.azure_storage_dfs_endpoint,
            "blob": logger.azure_storage_blob_endpoint,
        },
        "actions": results,
        "queue": [item["id"] for item in queue.log_queue],
        "calls": Wire.calls,
        "clients": {"created": Wire.created, "closed": Wire.closed},
    }


def run(input_value: Input) -> JsonValue:
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    results = [asyncio.run(drive(scenario, cache)) for scenario in input_value["scenarios"]]
    return {"results": results}

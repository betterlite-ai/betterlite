import asyncio
import io
import os
import sys
import time
from collections.abc import Mapping, Sequence
from datetime import timedelta
from types import ModuleType
from typing import ClassVar, Literal, NotRequired, Protocol, TypedDict

import azure.identity
import httpx
import litellm
import requests
import urllib3
from azure.core.pipeline.transport._requests_asyncio import AsyncioRequestsTransport
from azure.storage.filedatalake.aio import DataLakeServiceClient
from litellm.integrations.azure_storage.azure_storage import AzureBlobStorageLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, httpxSpecialProvider
from litellm.proxy import proxy_server
from litellm.types.utils import StandardLoggingPayload
from pydantic import JsonValue, TypeAdapter
from requests.adapters import HTTPAdapter
from typing_extensions import override

AIO_MODULE = "azure.storage.filedatalake.aio"
IDENTITY_MODULE = azure.identity.__name__
START_SECONDS = 1_700_000_000.0
BEARER = "tok-1"
JSON_VALUE = TypeAdapter(JsonValue)
SUCCESS_STATUS = {"create_directory": 201, "create_file": 201, "append": 202}
# LiteLLM's event methods need premium_user; a licensed setup has it, and betterlite never checks
proxy_server.premium_user = True


class Reply(TypedDict):
    status: int
    body: str


class Log(TypedDict):
    do: Literal["event", "upload"]
    id: str
    failure: NotRequired[bool]


class Control(TypedDict):
    do: Literal["flush", "advance"]
    seconds: NotRequired[int]


Action = Log | Control


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


class Uploader(Protocol):
    async def async_upload_payload_to_azure_blob_storage(  # ast-grep-ignore: no-long-names
        self, payload: StandardLoggingPayload
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


def operation(method: str, url: str) -> str:
    if method == "HEAD":
        return "exists"
    if "resource=directory" in url:
        return "create_directory"
    if "resource=file" in url:
        return "create_file"
    if "action=append" in url:
        return "append"
    if "action=flush" in url:
        return "flush"
    return "other"


def answer(method: str, url: str, body: bytes) -> tuple[int, str]:
    op = operation(method, url)
    today = time.strftime("%Y-%m-%d")
    entry: dict[str, JsonValue] = {"op": op, "url": url.replace(today, "{date}")}
    if op == "append":
        text = body.decode("utf-8")
        entry["body"] = JSON_VALUE.validate_json(text.rstrip("\n"))
        entry["trailing_newline"] = text.endswith("\n")
    Wire.calls.append(entry)
    if Wire.replies:
        reply = Wire.replies.pop(0)
        return reply["status"], reply["body"]
    return SUCCESS_STATUS[op] if op in SUCCESS_STATUS else 200, ""


def serve(request: httpx.Request) -> httpx.Response:
    status, text = answer(request.method, str(request.url), request.content)
    return httpx.Response(status, text=text, request=request)


class ScriptedAdapter(HTTPAdapter):
    @override
    def send(
        self,
        request: requests.PreparedRequest,
        stream: bool = False,
        timeout: float | tuple[float, float] | tuple[float, None] | None = None,
        verify: bool | str = True,
        cert: bytes | str | tuple[bytes | str, bytes | str] | None = None,
        proxies: Mapping[str, str] | None = None,
    ) -> requests.Response:
        body = request.body
        if body is None:
            body = b""
        match body:
            case bytes() as content:
                raw = content
            case str() as text:
                raw = text.encode("utf-8")
        status, text = answer(f"{request.method}", f"{request.url}", raw)
        payload = text.encode("utf-8")
        reply = urllib3.response.HTTPResponse(
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
    DataLakeServiceClient = ScriptedClient


class ScriptedIdentity(ModuleType):
    ClientSecretCredential = ScriptedCredential
    get_bearer_token_provider = ScriptedProvider


class NoCredentialChain:
    def __call__(self) -> ScriptedProvider:
        raise RuntimeError("no credential in the chain")


async def perform(
    action: Action,
    logger: AzureBlobStorageLogger,
    clock: ManualClock,
    base: dict[str, JsonValue],
) -> JsonValue:
    try:
        if action["do"] == "event":
            sink: EventSink = logger
            kwargs: dict[str, JsonValue] = {"standard_logging_object": {**base, "id": action["id"]}}
            if action.get("failure"):
                await sink.async_log_failure_event(kwargs, None, 0.0, 0.0)
            else:
                await sink.async_log_success_event(kwargs, None, 0.0, 0.0)
        if action["do"] == "upload":
            uploader: Uploader = logger
            payload = {**base, "id": action["id"]}
            await uploader.async_upload_payload_to_azure_blob_storage(payload)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
        if action["do"] == "flush":
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


async def drive(scenario: Scenario) -> JsonValue:
    apply_environment(scenario["environment"])
    replies = scenario.get("replies")
    Wire.calls = []
    Wire.replies = [] if replies is None else list(replies)
    Wire.created = 0
    Wire.closed = 0
    clock = ManualClock()
    try:
        logger = AzureBlobStorageLogger(
            build_credential_chain_token_provider=NoCredentialChain(), clock=clock
        )
    except ValueError as error:
        return {"initialized": False, "error": str(error)}
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    cache.set_cache(
        key="async_httpx_client" + httpxSpecialProvider.LoggingCallback,
        value=AsyncHTTPHandler(transport=httpx.MockTransport(serve)),
    )
    base = scenario.get("payload")
    base_payload = {} if base is None else base
    sys.modules[AIO_MODULE] = ScriptedAio(AIO_MODULE)
    sys.modules[IDENTITY_MODULE] = ScriptedIdentity(IDENTITY_MODULE)
    results = [await perform(action, logger, clock, base_payload) for action in scenario["actions"]]
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
    results = [asyncio.run(drive(scenario)) for scenario in input_value["scenarios"]]
    return {"results": results}

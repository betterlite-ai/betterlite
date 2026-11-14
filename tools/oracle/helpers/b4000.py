import asyncio
import io
import os
import sys
from datetime import datetime, timedelta
from types import ModuleType
from typing import ClassVar, Literal, NotRequired, Protocol, TypedDict

import azure.identity
import httpx
import litellm
import requests
import urllib3
from azure.core.pipeline.transport import AsyncioRequestsTransport
from azure.storage.filedatalake.aio import DataLakeServiceClient
from litellm.integrations.azure_storage.azure_storage import AzureBlobStorageLogger
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, httpxSpecialProvider
from litellm.types.utils import StandardLoggingPayload
from pydantic import JsonValue, TypeAdapter
from requests.adapters import HTTPAdapter
from typing_extensions import override

AIO_MODULE = "azure.storage.filedatalake.aio"
IDENTITY_MODULE = azure.identity.__name__
START_SECONDS = 1_700_000_000.0
TOKEN = "tok-1"
SUCCESS_STATUS = {"exists": 200, "create_directory": 201, "create_file": 201, "append": 202, "flush": 200}
JSON_VALUE = TypeAdapter(JsonValue)
PAYLOAD = TypeAdapter(StandardLoggingPayload)


class Reply(TypedDict):
    status: int
    body: NotRequired[str]


class Event(TypedDict):
    do: Literal["event"]
    id: str
    failure: NotRequired[bool]


class Upload(TypedDict):
    do: Literal["upload"]
    id: str


class Flush(TypedDict):
    do: Literal["flush"]


class Advance(TypedDict):
    do: Literal["advance"]
    seconds: int


Action = Event | Upload | Flush | Advance


class Scenario(TypedDict):
    environment: dict[str, str]
    actions: list[Action]
    payload: NotRequired[dict[str, JsonValue]]
    replies: NotRequired[list[Reply]]


class Input(TypedDict):
    scenarios: list[Scenario]


class Flusher(Protocol):
    async def flush_queue(self) -> None: ...


class Uploader(Protocol):
    async def async_upload_payload_to_azure_blob_storage(self, payload: StandardLoggingPayload) -> None: ...


class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler, **kwargs: int) -> None: ...


class Wire:
    # LiteLLM builds its HTTP client, credential and SDK client itself, so what they send and get back lives on the class
    calls: ClassVar[list[JsonValue]] = []
    replies: ClassVar[list[Reply]] = []
    created: ClassVar[int] = 0
    closed: ClassVar[int] = 0
    today: ClassVar[str] = ""


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
    entry: dict[str, JsonValue] = {"op": op, "url": url.replace(Wire.today, "{date}")}
    if op == "append":
        text = body.decode("utf-8")
        entry["body"] = JSON_VALUE.validate_json(text.rstrip("\n"))
        entry["trailing_newline"] = text.endswith("\n")
    Wire.calls.append(entry)
    if Wire.replies:
        reply = Wire.replies.pop(0)
        return reply["status"], reply["body"] if "body" in reply else ""
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
        proxies: dict[str, str] | None = None,
    ) -> requests.Response:
        raw = b""
        match request.body:
            case bytes() as content:
                raw = content
            case str() as text:
                raw = text.encode("utf-8")
            case None:
                raw = b""
        status, text = answer(f"{request.method}", f"{request.url}", raw)
        body = text.encode("utf-8")
        reply = urllib3.response.HTTPResponse(
            body=io.BytesIO(body),
            headers={"Content-Length": str(len(body))},
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
    def __init__(self, credential: "ScriptedCredential", scope: str) -> None:
        self.credential = credential
        self.scope = scope

    def __call__(self) -> str:
        Wire.calls.append({"op": "entra_token"})
        return TOKEN


class ScriptedCredential:
    def __init__(self, tenant_id: str, client_id: str, client_secret: str) -> None:
        self.tenant_id = tenant_id
        self.client_id = client_id
        self.client_secret = client_secret


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
    uploader: Uploader = logger
    flusher: Flusher = logger
    try:
        if action["do"] == "event":
            logger.log_queue.append(PAYLOAD.validate_python({**base, "id": action["id"]}))
        if action["do"] == "upload":
            await uploader.async_upload_payload_to_azure_blob_storage(PAYLOAD.validate_python({**base, "id": action["id"]}))
        if action["do"] == "flush":
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
    Wire.today = datetime.now().strftime("%Y-%m-%d")
    clock = ManualClock()
    try:
        logger = AzureBlobStorageLogger(build_credential_chain_token_provider=NoCredentialChain(), clock=clock)
    except ValueError as error:
        return {"initialized": False, "error": str(error)}
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    cache.set_cache(
        key="async_httpx_client" + httpxSpecialProvider.LoggingCallback,
        value=AsyncHTTPHandler(transport=httpx.MockTransport(serve)),
    )
    base = scenario.get("payload")
    results: list[JsonValue] = []
    original_aio = sys.modules[AIO_MODULE]
    original_identity = sys.modules[IDENTITY_MODULE]
    sys.modules[AIO_MODULE] = ScriptedAio(AIO_MODULE)
    sys.modules[IDENTITY_MODULE] = ScriptedIdentity(IDENTITY_MODULE)
    try:
        for action in scenario["actions"]:
            results.append(await perform(action, logger, clock, {} if base is None else base))
    finally:
        sys.modules[AIO_MODULE] = original_aio
        sys.modules[IDENTITY_MODULE] = original_identity
    return {
        "initialized": True,
        "endpoints": {"dfs": logger.azure_storage_dfs_endpoint, "blob": logger.azure_storage_blob_endpoint},
        "actions": results,
        "queue": [payload["id"] for payload in logger.log_queue],
        "calls": Wire.calls,
        "clients": {"created": Wire.created, "closed": Wire.closed},
    }


async def drive_all(scenarios: list[Scenario]) -> JsonValue:
    results: list[JsonValue] = []
    for scenario in scenarios:
        results.append(await drive(scenario))
    return {"results": results}


def run(input_value: Input) -> JsonValue:
    return asyncio.run(drive_all(input_value["scenarios"]))

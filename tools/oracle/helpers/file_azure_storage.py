import asyncio
import sys
import time
from types import ModuleType
from typing import Final, Protocol, TypedDict

import azure.identity
import fastuuid
import httpx
import litellm
from fastuuid import UUID
from litellm.llms.base_llm.files.storage_backend_factory import get_storage_backend
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, httpxSpecialProvider
from pydantic import JsonValue

BEARER = "tok-1"
CLIENT_KEY: Final = "async_httpx_client" + httpxSpecialProvider.LoggingCallback
IDENTITY_MODULE = azure.identity.__name__


class FileNameCall(TypedDict):
    filename: str
    strategy: str
    now_seconds: float
    uuid: str


class Input(TypedDict):
    calls: list[FileNameCall]


class Clock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = UUID(value)

    def __call__(self) -> UUID:
        return self.value


class AnswerStatus:
    def __call__(self, request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, request=request)


class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler) -> None: ...


class ScriptedCredential:
    def __init__(self, *_args: "ScriptedCredential", **_kwargs: str | None) -> None: ...


class ScriptedProvider:
    def __init__(self, _credential: ScriptedCredential, _scope: str) -> None: ...

    def __call__(self) -> str:
        return BEARER


class ScriptedIdentity(ModuleType):
    CertificateCredential: type[ScriptedCredential] = ScriptedCredential
    ChainedTokenCredential: type[ScriptedCredential] = ScriptedCredential
    ClientSecretCredential: type[ScriptedCredential] = ScriptedCredential
    DefaultAzureCredential: type[ScriptedCredential] = ScriptedCredential
    ManagedIdentityCredential: type[ScriptedCredential] = ScriptedCredential
    WorkloadIdentityCredential: type[ScriptedCredential] = ScriptedCredential
    get_bearer_token_provider: type[ScriptedProvider] = ScriptedProvider


async def generate_name(call: FileNameCall, cache: ClientCache) -> JsonValue:
    time.time = Clock(call["now_seconds"])
    fastuuid.uuid4 = FixedUuid(call["uuid"])
    cache.set_cache(
        key=CLIENT_KEY, value=AsyncHTTPHandler(transport=httpx.MockTransport(AnswerStatus()))
    )
    backend = get_storage_backend("azure_storage")
    url = await backend.upload_file(
        file_content=b"abc",
        filename=call["filename"],
        content_type="text/plain",
        path_prefix=None,
        file_naming_strategy=call["strategy"],
    )
    name: dict[str, JsonValue] = {"name": url.rsplit("/", 1)[-1]}
    return name


def run(input_value: Input) -> list[JsonValue]:
    sys.modules[IDENTITY_MODULE] = ScriptedIdentity(IDENTITY_MODULE)
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    names: list[JsonValue] = [asyncio.run(generate_name(call, cache)) for call in input_value["calls"]]
    return names

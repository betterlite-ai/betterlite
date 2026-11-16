import asyncio
import time
from typing import Final, Protocol, TypedDict

import fastuuid
import httpx
import litellm
from fastuuid import UUID
from litellm.integrations.azure_storage.azure_storage import AzureBlobStorageLogger
from litellm.llms.base_llm.files.azure_blob_storage_backend import AzureBlobStorageBackend
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, httpxSpecialProvider
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
BEARER = "tok-1"
CLIENT_KEY: Final = "async_httpx_client" + httpxSpecialProvider.LoggingCallback


class FileNameCall(TypedDict):
    filename: str
    strategy: str
    now_seconds: float
    uuid: str


class Input(TypedDict):
    calls: list[FileNameCall]


class FixedToken:
    def __call__(self) -> str:
        return BEARER


class FixedChain:
    def __call__(self) -> FixedToken:
        return FixedToken()


class Clock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


class FixedFast:
    def __init__(self, value: str) -> None:
        self.value = UUID(value)

    def uuid4(self) -> UUID:
        return self.value


class AnswerStatus:
    def __call__(self, request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, request=request)


class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler) -> None: ...


class Backend(AzureBlobStorageBackend):
    def __init__(self) -> None:
        AzureBlobStorageLogger.__init__(self, build_credential_chain_token_provider=FixedChain())


async def generate_name(call: FileNameCall) -> JsonValue:
    time.time = Clock(call["now_seconds"])
    fastuuid.uuid4 = FixedFast(call["uuid"]).uuid4
    cache: ClientCache = litellm.in_memory_llm_clients_cache
    cache.set_cache(
        key=CLIENT_KEY, value=AsyncHTTPHandler(transport=httpx.MockTransport(AnswerStatus()))
    )
    backend = Backend()
    url = await backend.upload_file(
        file_content=b"abc",
        filename=call["filename"],
        content_type="text/plain",
        path_prefix=None,
        file_naming_strategy=call["strategy"],
    )
    return JSON.validate_python({"name": url.rsplit("/", 1)[-1]})


async def generate_names(input_value: Input) -> list[JsonValue]:
    names: list[JsonValue] = []
    for call in input_value["calls"]:
        names.append(await generate_name(call))
    return names


def run(input_value: Input) -> JsonValue:
    names = asyncio.run(generate_names(input_value))
    return JSON.validate_python(names)

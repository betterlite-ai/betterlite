import asyncio
import time
from collections.abc import Callable
from typing import Final, TypedDict
from unittest.mock import patch

import fastuuid
import httpx
from litellm.llms.base_llm.files.azure_blob_storage_backend import AzureBlobStorageBackend
from litellm.llms.custom_httpx import http_handler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class FileNameCall(TypedDict):
    filename: str
    strategy: str
    now_seconds: float
    uuid: str


class Input(TypedDict):
    calls: list[FileNameCall]


def fixed_token() -> str:
    return "tok-1"


def build_token_provider() -> Callable[[], str]:
    return fixed_token


class Clock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


class FixedFast:
    def __init__(self, value: str) -> None:
        self.value = fastuuid.UUID(value)

    def __call__(self) -> fastuuid.UUID:
        return self.value


def answer_status(request: httpx.Request) -> httpx.Response:
    return httpx.Response(200, request=request)


def async_handler(**options: JsonValue) -> AsyncHTTPHandler:
    del options
    return AsyncHTTPHandler(transport=httpx.MockTransport(answer_status))


async def generate_name(call: FileNameCall) -> JsonValue:
    time.time = Clock(call["now_seconds"])
    fastuuid.uuid4 = FixedFast(call["uuid"])
    backend = AzureBlobStorageBackend(
        build_credential_chain_token_provider=build_token_provider
    )
    with patch.object(http_handler, "get_async_httpx_client", async_handler):
        url = await backend.upload_file(
            file_content=b"abc",
            filename=call["filename"],
            content_type="text/plain",
            path_prefix=None,
            file_naming_strategy=call["strategy"],
        )
    return JSON.validate_python({"name": url.rsplit("/", 1)[-1]})


async def collect(input_value: Input) -> list[JsonValue]:
    return [await generate_name(call) for call in input_value["calls"]]


def run(input_value: Input) -> JsonValue:
    return JSON.validate_python(asyncio.run(collect(input_value)))

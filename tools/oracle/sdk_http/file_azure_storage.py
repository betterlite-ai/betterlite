import asyncio
import json
from typing import Literal

import httpx
from azure.core.exceptions import HttpResponseError
from litellm.llms.base_llm.files.azure_blob_storage_backend import AzureBlobStorageBackend
from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import override

JsonObject = dict[str, JsonValue]
BEARER = "tok-1"


class UploadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["upload"]
    file_content: list[int]
    filename: str
    content_type: str
    path_prefix: str | None
    file_naming_strategy: str


class DownloadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["download"]
    storage_url: str


FileAzureStorageCall = UploadCall | DownloadCall


class FixedToken:
    def __call__(self) -> str:
        return BEARER


class FixedChain:
    def __call__(self) -> FixedToken:
        return FixedToken()


class Backend(AzureBlobStorageBackend):
    api_base: str

    # the SDK rewrites ".dfs." to ".blob.", so the mock's prefixes keep both origins apart
    @property
    @override
    def azure_storage_dfs_endpoint(self) -> str:
        return f"{self.api_base}/.dfs."

    @property
    @override
    def azure_storage_blob_endpoint(self) -> str:
        return f"{self.api_base}/.blob."


def error_view(error: Exception, status: int | None) -> JsonObject:
    return {
        "url": None,
        "downloaded": None,
        "error": {"message": str(error), "status": status},
    }


async def upload_view(backend: Backend, call: UploadCall) -> JsonObject:
    url = await backend.upload_file(
        file_content=bytes(call.file_content),
        filename=call.filename,
        content_type=call.content_type,
        path_prefix=call.path_prefix,
        file_naming_strategy=call.file_naming_strategy,
    )
    return {"url": url.removeprefix(backend.api_base), "downloaded": None, "error": None}


async def download_view(backend: Backend, call: DownloadCall) -> JsonObject:
    content = await backend.download_file(call.storage_url)
    return {"url": None, "downloaded": list(content), "error": None}


async def execute(call: FileAzureStorageCall, api_base: str) -> JsonObject:
    backend = Backend(build_credential_chain_token_provider=FixedChain())
    backend.api_base = api_base
    try:
        match call:
            case UploadCall():
                return await upload_view(backend, call)
            case DownloadCall():
                return await download_view(backend, call)
    except ValueError as error:
        return error_view(error, None)
    except HttpResponseError as error:
        return error_view(error, error.status_code)
    except httpx.HTTPStatusError as error:
        return error_view(error, error.response.status_code)


def run_file_azure_storage(call: FileAzureStorageCall, api_base: str) -> str:
    value = asyncio.run(execute(call, api_base))
    return json.dumps({"file_azure_storage": {"value": value}}, separators=(",", ":"))

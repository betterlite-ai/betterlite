import asyncio
import time
from typing import Literal, TypedDict

import httpx
from litellm.integrations.azure_storage.azure_storage import AzureBlobStorageLogger
from litellm.llms.base_llm.files.azure_blob_storage_backend import AzureBlobStorageBackend
from pydantic import BaseModel, ConfigDict, TypeAdapter
from typing_extensions import override

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


class FileStorageFailure(TypedDict):
    message: str
    status: int | None


class FileStorageRun(TypedDict):
    url: str | None
    downloaded: list[int] | None
    error: FileStorageFailure | None


class FileAzureStorageValue(TypedDict):
    value: FileStorageRun


class FileAzureStorageOutcome(TypedDict):
    file_azure_storage: FileAzureStorageValue


class FixedToken:
    def __call__(self) -> str:
        return BEARER


class FixedChain:
    def __call__(self) -> FixedToken:
        return FixedToken()


class Backend(AzureBlobStorageBackend):
    api_base: str

    # the backend's __init__ is untyped, so the annotated logger one parses the environment
    def __init__(self, api_base: str) -> None:
        source = AzureBlobStorageLogger()
        self.tenant_id = source.tenant_id
        self.client_id = source.client_id
        self.client_secret = source.client_secret
        self.azure_storage_account_key = source.azure_storage_account_key
        self.azure_storage_account_name = source.azure_storage_account_name
        self.azure_storage_file_system = source.azure_storage_file_system
        self.azure_storage_endpoint_suffix = source.azure_storage_endpoint_suffix
        self.azure_auth_token = None
        self.token_expiry = None
        self._service_client = None
        self._service_client_timeout = None
        self._clock = time.time
        self._build_credential_chain_token_provider = FixedChain()
        self.api_base = api_base

    # the SDK rewrites ".dfs." to ".blob.", so the mock's prefixes keep both origins apart
    @property
    @override
    def azure_storage_dfs_endpoint(self) -> str:
        return f"{self.api_base}/.dfs."

    @property
    @override
    def azure_storage_blob_endpoint(self) -> str:
        return f"{self.api_base}/.blob."


def error_view(error: Exception, status: int | None) -> FileStorageRun:
    return {
        "url": None,
        "downloaded": None,
        "error": {"message": str(error), "status": status},
    }


async def upload_view(backend: Backend, call: UploadCall) -> FileStorageRun:
    url = await backend.upload_file(
        file_content=bytes(call.file_content),
        filename=call.filename,
        content_type=call.content_type,
        path_prefix=call.path_prefix,
        file_naming_strategy=call.file_naming_strategy,
    )
    return {"url": url.removeprefix(backend.api_base), "downloaded": None, "error": None}


async def download_view(backend: Backend, call: DownloadCall) -> FileStorageRun:
    content = await backend.download_file(call.storage_url)
    return {"url": None, "downloaded": list(content), "error": None}


async def execute(call: FileAzureStorageCall, api_base: str) -> FileStorageRun:
    value: FileStorageRun
    try:
        backend = Backend(api_base)
        match call:
            case UploadCall():
                value = await upload_view(backend, call)
            case DownloadCall():
                value = await download_view(backend, call)
    except ValueError as error:
        return error_view(error, None)
    except httpx.HTTPStatusError as error:
        return error_view(error, error.response.status_code)
    return value


def run_file_azure_storage(call: FileAzureStorageCall, api_base: str) -> str:
    value = asyncio.run(execute(call, api_base))
    outcome: FileAzureStorageOutcome = {"file_azure_storage": {"value": value}}
    return TypeAdapter(FileAzureStorageOutcome).dump_json(outcome).decode()

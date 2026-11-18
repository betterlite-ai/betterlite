import asyncio
from typing import Literal, TypedDict

import fastuuid
import httpx
from litellm.llms.base_llm.files.storage_backend_factory import get_storage_backend
from pydantic import BaseModel, ConfigDict, TypeAdapter

from azure_file_storage import Backend, FileStorageRun, error_view


class FactoryCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["factory"]
    backend_type: str
    target_model_names: list[str]


class UploadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["upload"]
    file_content: list[int]
    filename: str
    content_type: str
    target_storage: str
    target_model_names: list[str]
    uuid: str


FileStorageServiceCall = FactoryCall | UploadCall


class FileStorageServiceValue(TypedDict):
    value: FileStorageRun


class FileStorageServiceOutcome(TypedDict):
    file_storage_service: FileStorageServiceValue


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> fastuuid.UUID:
        return fastuuid.UUID(hex=self.value)


async def upload_view(call: UploadCall, api_base: str) -> FileStorageRun:
    fastuuid.uuid4 = FixedUuid(call.uuid)
    backend = Backend(api_base)
    url = await backend.upload_file(
        file_content=bytes(call.file_content),
        filename=call.filename,
        content_type=call.content_type,
        path_prefix="",
        file_naming_strategy="uuid",
    )
    return {"url": url.removeprefix(api_base), "downloaded": None, "error": None}


async def execute(call: FileStorageServiceCall, api_base: str) -> FileStorageRun:
    value: FileStorageRun
    try:
        match call:
            case FactoryCall():
                _ = get_storage_backend(call.backend_type, prisma_client=None)
                value = {"url": None, "downloaded": None, "error": None}
            case UploadCall():
                value = await upload_view(call, api_base)
    except ValueError as error:
        return error_view(error, None)
    except httpx.HTTPStatusError as error:
        return error_view(error, error.response.status_code)
    return value


def run_file_storage_service(call: FileStorageServiceCall, api_base: str) -> str:
    run = asyncio.run(execute(call, api_base))
    outcome: FileStorageServiceOutcome = {"file_storage_service": {"value": run}}
    return TypeAdapter(FileStorageServiceOutcome).dump_json(outcome).decode()


class FileStorageServiceRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_storage_service: FileStorageServiceCall

    def serialize(self, api_base: str) -> str:
        return run_file_storage_service(self.file_storage_service, api_base)

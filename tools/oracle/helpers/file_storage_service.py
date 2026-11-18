import time
from typing import Final, TypedDict

import fastuuid
from litellm.proxy.openai_files_endpoints.storage_backend_service import (
    StorageBackendFileService,
)
from litellm.types.llms.openai import OpenAIFilesPurpose
from pydantic import TypeAdapter


class ProjectedFile(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: OpenAIFilesPurpose
    status: str | None


PROJECTED: Final = TypeAdapter(ProjectedFile)


class Input(TypedDict):
    content: str
    filename: str
    purpose: OpenAIFilesPurpose
    target_storage: str
    storage_url: str
    now: float
    uuid: str


class FixedClock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def __call__(self) -> fastuuid.UUID:
        return fastuuid.UUID(hex=self.value)


def run(input_value: Input) -> ProjectedFile:
    time.time = FixedClock(input_value["now"])
    fastuuid.uuid4 = FixedUuid(input_value["uuid"])
    file_object = StorageBackendFileService._create_file_object_with_storage_metadata(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/storage_backend_service.py#L141
        file_content=input_value["content"].encode(),
        filename=input_value["filename"],
        purpose=input_value["purpose"],
        target_storage=input_value["target_storage"],
        storage_url=input_value["storage_url"],
    )
    return PROJECTED.validate_json(file_object.model_dump_json())

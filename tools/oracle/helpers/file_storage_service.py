from functools import partial
from typing import Final, TypedDict
from unittest import mock

from fastuuid import UUID
from litellm._uuid import uuid as uuid_module
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


def run(input_value: Input) -> ProjectedFile:
    with (
        mock.patch("time.time", partial(float, input_value["now"])),
        mock.patch.object(uuid_module, "uuid4", partial(UUID, hex=input_value["uuid"])),
    ):
        file_object = StorageBackendFileService._create_file_object_with_storage_metadata(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/storage_backend_service.py#L141
            file_content=input_value["content"].encode(),
            filename=input_value["filename"],
            purpose=input_value["purpose"],
            target_storage=input_value["target_storage"],
            storage_url=input_value["storage_url"],
        )
    return PROJECTED.validate_json(file_object.model_dump_json())

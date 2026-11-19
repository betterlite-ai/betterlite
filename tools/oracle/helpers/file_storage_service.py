import asyncio
import os
import time
from typing import Final, Literal, TypedDict, cast

import fastuuid
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.openai_files_endpoints.storage_backend_service import (
    StorageBackendFileService,
)
from litellm.proxy.utils import ProxyLogging
from litellm.types.llms.openai import OpenAIFileObject, OpenAIFilesPurpose
from pydantic import JsonValue, TypeAdapter


class ProjectedFile(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: OpenAIFilesPurpose
    status: str | None


PROJECTED: Final = TypeAdapter(ProjectedFile)


class HiddenParams(TypedDict):
    storage_backend: str
    storage_url: str


HIDDEN: Final = TypeAdapter(HiddenParams)


class Projection(TypedDict):
    file: ProjectedFile
    storage_backend: str
    storage_url: str


class ErrorBody(TypedDict):
    message: str
    type: str
    param: str | None
    code: str


class Refusal(TypedDict):
    error: ErrorBody


class Input(TypedDict):
    operation: Literal["projection", "upload"]
    content: str
    filename: str
    content_type: str
    purpose: OpenAIFilesPurpose
    target_storage: str
    storage_url: str
    target_model_names: list[str]
    environment: dict[str, str]
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


def project(file_object: OpenAIFileObject) -> Projection:
    hidden_params: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/storage_backend_service.py#L172-L180
        dict[str, JsonValue],
        file_object._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/storage_backend_service.py#L172-L180
    )
    hidden = HIDDEN.validate_python(hidden_params)
    return {
        "file": PROJECTED.validate_json(file_object.model_dump_json()),
        "storage_backend": hidden["storage_backend"],
        "storage_url": hidden["storage_url"],
    }


async def upload_view(input_value: Input) -> Projection | Refusal:
    proxy_logging_obj = ProxyLogging(user_api_key_cache=UserApiKeyCache())
    try:
        file_object = await StorageBackendFileService.upload_file_to_storage_backend(
            file_data={
                "content": input_value["content"].encode(),
                "filename": input_value["filename"],
                "content_type": input_value["content_type"],
                # extract_file_data gives an upload without headers an empty mapping
                "headers": {},
            },
            target_storage=input_value["target_storage"],
            target_model_names=input_value["target_model_names"],
            purpose=input_value["purpose"],
            proxy_logging_obj=proxy_logging_obj,
            user_api_key_dict=UserAPIKeyAuth(),
        )
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {
            "error": {
                "message": error.message,
                "type": error.type,
                "param": error.param,
                "code": error.code,
            }
        }
    return project(file_object)


def run(input_value: Input) -> Projection | Refusal:
    os.environ.update(input_value["environment"])
    time.time = FixedClock(input_value["now"])
    fastuuid.uuid4 = FixedUuid(input_value["uuid"])
    if input_value["operation"] == "projection":
        file_object = StorageBackendFileService._create_file_object_with_storage_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/storage_backend_service.py#L141
            file_content=input_value["content"].encode(),
            filename=input_value["filename"],
            purpose=input_value["purpose"],
            target_storage=input_value["target_storage"],
            storage_url=input_value["storage_url"],
        )
        return project(file_object)
    return asyncio.run(upload_view(input_value))

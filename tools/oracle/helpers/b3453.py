import asyncio
import base64
import importlib
from functools import partial
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
import litellm.vector_store_files.main as vector_store_files_main
import litellm.vector_stores.main as vector_stores_main
from litellm.rag.ingestion import openai_ingestion
from litellm.types.llms.openai import OpenAIFileObject
from litellm.types.rag import RAGIngestOptions
from litellm.types.vector_stores import VectorStoreCreateResponse
from pydantic import JsonValue


class CreateResponseInput(TypedDict):
    id: NotRequired[str]


class StoreInput(TypedDict):
    options: RAGIngestOptions
    file_content_base64: NotRequired[str]
    filename: NotRequired[str]
    content_type: NotRequired[str]
    existing_file_id: NotRequired[str]
    create_response: NotRequired[CreateResponseInput]
    create_error: NotRequired[str]
    attach_error: NotRequired[str]
    upload_id: NotRequired[str]
    upload_error: NotRequired[str]


class Script(TypedDict):
    create_id: str | None
    create_error: str | None
    attach_error: str | None
    upload_id: str
    upload_error: str | None


class CreateCallView(TypedDict):
    port: Literal["vector_store_create"]
    name: str
    expires_after: dict[str, JsonValue] | None
    api_key: bool
    api_base: str | None


class AttachCallView(TypedDict):
    port: Literal["vector_file_attach"]
    vector_store_id: str
    file_id: str
    chunking_strategy: dict[str, JsonValue] | None
    api_key: bool
    api_base: JsonValue


class UploadCallView(TypedDict):
    port: Literal["file_upload"]
    filename: str
    size_bytes: int
    content_type: str
    purpose: str
    api_key: bool
    api_base: str | None


CallView: TypeAlias = CreateCallView | AttachCallView | UploadCallView


class StoreView(TypedDict):
    calls: list[CallView]
    result: list[str | None]


class CallRecorder:
    def __init__(self, script: Script) -> None:
        self._script = script
        self.calls: list[CallView] = []

    def _guard(self, error: str | None) -> None:
        if error is not None:
            raise RuntimeError(error)

    async def create(
        self,
        *,
        name: str,
        custom_llm_provider: str | None = None,
        expires_after: dict[str, JsonValue] | None = None,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> VectorStoreCreateResponse:
        del custom_llm_provider
        self.calls.append(
            CreateCallView(
                port="vector_store_create",
                name=name,
                expires_after=expires_after,
                api_key=api_key is not None,
                api_base=api_base,
            )
        )
        self._guard(self._script["create_error"])
        create_id = self._script["create_id"]
        if create_id is None:
            return VectorStoreCreateResponse()
        return VectorStoreCreateResponse(id=create_id)

    async def attach(
        self,
        *,
        vector_store_id: str,
        file_id: str,
        chunking_strategy: dict[str, JsonValue] | None,
        api_key: str | None = None,
        **kwargs: JsonValue,
    ) -> None:
        self.calls.append(
            AttachCallView(
                port="vector_file_attach",
                vector_store_id=vector_store_id,
                file_id=file_id,
                chunking_strategy=chunking_strategy,
                api_key=api_key is not None,
                api_base=kwargs.get("api_base"),
            )
        )
        self._guard(self._script["attach_error"])

    async def upload(
        self,
        *,
        file: tuple[str, bytes, str],
        purpose: str,
        custom_llm_provider: str | None = None,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> OpenAIFileObject:
        del custom_llm_provider
        filename, content, content_type = file
        self.calls.append(
            UploadCallView(
                port="file_upload",
                filename=filename,
                size_bytes=len(content),
                content_type=content_type,
                purpose=purpose,
                api_key=api_key is not None,
                api_base=api_base,
            )
        )
        self._guard(self._script["upload_error"])
        return OpenAIFileObject(
            id=self._script["upload_id"],
            bytes=len(content),
            created_at=0,
            filename=filename,
            object="file",
            purpose="assistants",
        )


def _script(input_value: StoreInput) -> Script:
    create_response = input_value.get("create_response")
    create_id = None if create_response is None else create_response.get("id")
    upload_option = input_value.get("upload_id")
    upload_id = "file-uploaded" if upload_option is None else upload_option
    return Script(
        create_id=create_id,
        create_error=input_value.get("create_error"),
        attach_error=input_value.get("attach_error"),
        upload_id=upload_id,
        upload_error=input_value.get("upload_error"),
    )


def run(input_value: StoreInput) -> StoreView:
    recorder = CallRecorder(_script(input_value))
    vector_stores_main.acreate = partial(recorder.create)
    vector_store_files_main.acreate = partial(recorder.attach)
    litellm.acreate_file = partial(recorder.upload)
    importlib.reload(openai_ingestion)
    instance = openai_ingestion.OpenAIRAGIngestion(input_value["options"])
    encoded = input_value.get("file_content_base64")
    content = None if encoded is None else base64.b64decode(encoded)
    stored = asyncio.run(
        instance.store(
            file_content=content,
            filename=input_value.get("filename"),
            content_type=input_value.get("content_type"),
            chunks=[],
            embeddings=None,
            existing_file_id=input_value.get("existing_file_id"),
        )
    )
    return StoreView(calls=recorder.calls, result=[stored[0], stored[1]])

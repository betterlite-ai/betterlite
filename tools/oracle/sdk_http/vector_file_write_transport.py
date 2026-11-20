import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.vector_store_files.transformation import (
    BaseVectorStoreFilesConfig,
)
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.vector_store_files.transformation import (
    OpenAIVectorStoreFilesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_store_files import (
    VectorStoreFileDeleteResponse,
    VectorStoreFileObject,
    VectorStoreFileUpdateRequest,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["update", "delete"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
# the handlers take dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

UpdateWork = VectorStoreFileObject | Coroutine[object, object, VectorStoreFileObject]
DeleteWork = (
    VectorStoreFileDeleteResponse | Coroutine[object, object, VectorStoreFileDeleteResponse]
)
UpdateAnswer = Coroutine[object, object, VectorStoreFileObject]
DeleteAnswer = Coroutine[object, object, VectorStoreFileDeleteResponse]

UpdateKwargs = TypedDict(
    "UpdateKwargs",
    {
        "vector_store_id": str,
        "file_id": str,
        "update_request": VectorStoreFileUpdateRequest,
        "vector_store_files_provider_config": BaseVectorStoreFilesConfig,
        "custom_llm_provider": str,
        "litellm_params": GenericLiteLLMParams,
        "logging_obj": Logging,
        "extra_headers": dict[str, str] | None,
        "extra_body": dict[str, object] | None,
        "timeout": float | httpx.Timeout | None,
    },
)

DeleteKwargs = TypedDict(
    "DeleteKwargs",
    {
        "vector_store_id": str,
        "file_id": str,
        "vector_store_files_provider_config": BaseVectorStoreFilesConfig,
        "custom_llm_provider": str,
        "litellm_params": GenericLiteLLMParams,
        "logging_obj": Logging,
        "extra_headers": dict[str, str] | None,
        "timeout": float | httpx.Timeout | None,
    },
)


class UpdateHandler(Protocol):
    def __call__(self, **kwargs: Unpack[UpdateKwargs]) -> UpdateWork: ...


class AsyncUpdateHandler(Protocol):
    def __call__(self, **kwargs: Unpack[UpdateKwargs]) -> UpdateAnswer: ...


class DeleteHandler(Protocol):
    def __call__(self, **kwargs: Unpack[DeleteKwargs]) -> DeleteWork: ...


class AsyncDeleteHandler(Protocol):
    def __call__(self, **kwargs: Unpack[DeleteKwargs]) -> DeleteAnswer: ...


class Handlers(Protocol):
    @property
    def vector_store_file_update_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> UpdateHandler: ...

    @property
    def async_vector_store_file_update_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncUpdateHandler: ...

    @property
    def vector_store_file_delete_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> DeleteHandler: ...

    @property
    def async_vector_store_file_delete_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncDeleteHandler: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


class UpdateFields(BaseModel):
    model_config = ConfigDict(extra="forbid")

    attributes: dict[str, str | int | float | bool] | None = None


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    vector_store_id: str
    file_id: str
    update_request: UpdateFields = Field(default_factory=UpdateFields)
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: int | None = None


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class Outcome(BaseModel):
    result: JsonValue = None
    failure: Failure | None = None


class TransportValue(BaseModel):
    value: Outcome


class TransportEnvelope(BaseModel):
    vector_file_write_transport: TransportValue


def logging_fixture(operation: Operation) -> Logging:
    call_type = f"vector_store_file_{operation}"
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=call_type,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="vector-file-write-transport",
        function_id="vector-file-write-transport",
    )


def requested_update(fields: UpdateFields) -> VectorStoreFileUpdateRequest:
    request: VectorStoreFileUpdateRequest = {}
    if fields.attributes is not None:
        request["attributes"] = fields.attributes
    return request


def execute(
    call: TransportCall,
    config: BaseVectorStoreFilesConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture(call.operation)
    litellm_params = GenericLiteLLMParams(api_base=api_base)
    body = OBJECTS.validate_python(call.extra_body)
    update = requested_update(call.update_request)
    if call.operation == "update":
        if call.mode == "sync":
            return JSON.validate_python(
                HANDLER.vector_store_file_update_handler(
                    vector_store_id=call.vector_store_id,
                    file_id=call.file_id,
                    update_request=update,
                    vector_store_files_provider_config=config,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    extra_headers=call.extra_headers,
                    extra_body=body,
                    timeout=timeout,
                )
            )
        return JSON.validate_python(
            asyncio.run(
                HANDLER.async_vector_store_file_update_handler(
                    vector_store_id=call.vector_store_id,
                    file_id=call.file_id,
                    update_request=update,
                    vector_store_files_provider_config=config,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    extra_headers=call.extra_headers,
                    extra_body=body,
                    timeout=timeout,
                )
            )
        )
    if call.mode == "sync":
        return JSON.validate_python(
            HANDLER.vector_store_file_delete_handler(
                vector_store_id=call.vector_store_id,
                file_id=call.file_id,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=litellm_params,
                logging_obj=logging_obj,
                extra_headers=call.extra_headers,
                timeout=timeout,
            )
        )
    return JSON.validate_python(
        asyncio.run(
            HANDLER.async_vector_store_file_delete_handler(
                vector_store_id=call.vector_store_id,
                file_id=call.file_id,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=litellm_params,
                logging_obj=logging_obj,
                extra_headers=call.extra_headers,
                timeout=timeout,
            )
        )
    )


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config: BaseVectorStoreFilesConfig = OpenAIVectorStoreFilesConfig()
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        result = execute(call, config, timeout, api_base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


class FileWriteTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_write_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.vector_file_write_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(vector_file_write_transport=value).model_dump_json(
            by_alias=True
        )

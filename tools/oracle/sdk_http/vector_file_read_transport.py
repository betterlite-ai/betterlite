import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, TypeVar, Unpack

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
    VectorStoreFileContentResponse,
    VectorStoreFileStatus,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["retrieve", "content"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
# VectorStoreFileObject's last_error is dict[str, Any] | None, and mypy rejects every expression
# of a TypedDict carrying Any, so the retrieve result names its other fields instead
class ReadFileObject(TypedDict, total=False):
    id: str
    created_at: int
    usage_bytes: int | None
    vector_store_id: str
    status: VectorStoreFileStatus
    attributes: dict[str, str | int | float | bool] | None

RetrieveResult: TypeAlias = ReadFileObject | Coroutine[object, object, ReadFileObject]
AsyncRetrieveResult: TypeAlias = Coroutine[object, object, ReadFileObject]
ContentResult: TypeAlias = (
    VectorStoreFileContentResponse | Coroutine[object, object, VectorStoreFileContentResponse]
)
AsyncContentResult: TypeAlias = Coroutine[object, object, VectorStoreFileContentResponse]
Result_co = TypeVar("Result_co", covariant=True)


class ReadArguments(TypedDict):
    vector_store_id: str
    file_id: str
    # ast-grep-ignore: no-long-names
    vector_store_files_provider_config: BaseVectorStoreFilesConfig
    custom_llm_provider: str
    litellm_params: GenericLiteLLMParams
    logging_obj: Logging
    extra_headers: dict[str, str]
    timeout: float | httpx.Timeout | None


class ReadHandler(Protocol[Result_co]):
    def __call__(self, **kwargs: Unpack[ReadArguments]) -> Result_co: ...


class ReadHandlers(Protocol):
    @property
    def vector_store_file_retrieve_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> ReadHandler[RetrieveResult]: ...

    @property
    def async_vector_store_file_retrieve_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> ReadHandler[AsyncRetrieveResult]: ...

    @property
    def vector_store_file_content_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> ReadHandler[ContentResult]: ...

    @property
    def async_vector_store_file_content_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> ReadHandler[AsyncContentResult]: ...


HANDLER: ReadHandlers = BaseLLMHTTPHandler()


class ReadCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    vector_store_id: str
    file_id: str
    operations: list[Operation]
    extra_headers: dict[str, str] | None = None
    timeout_seconds: int | None = None


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str]


class Outcome(BaseModel):
    results: list[JsonValue] = Field(default_factory=list)
    failure: Failure | None = None


class ReadValue(BaseModel):
    value: Outcome


class ReadEnvelope(BaseModel):
    vector_file_read_transport: ReadValue


def logging_fixture(operation: Operation) -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=f"vector_store_file_{operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="vector-file-read-transport",
        function_id="vector-file-read-transport",
    )


def caller_headers(call: ReadCall) -> dict[str, str]:
    if call.extra_headers is None:
        return {}
    return dict(call.extra_headers)


def read_timeout(call: ReadCall) -> float | httpx.Timeout | None:
    if call.timeout_seconds is None:
        return None
    return float(call.timeout_seconds)


def read_arguments(call: ReadCall, operation: Operation, api_base: str) -> ReadArguments:
    return {
        "vector_store_id": call.vector_store_id,
        "file_id": call.file_id,
        "vector_store_files_provider_config": OpenAIVectorStoreFilesConfig(),
        "custom_llm_provider": "openai",
        "litellm_params": GenericLiteLLMParams(api_base=api_base),
        "logging_obj": logging_fixture(operation),
        "extra_headers": caller_headers(call),
        "timeout": read_timeout(call),
    }


def sync_handler(operation: Operation) -> ReadHandler[RetrieveResult | ContentResult]:
    if operation == "retrieve":
        return HANDLER.vector_store_file_retrieve_handler
    return HANDLER.vector_store_file_content_handler


def async_handler(operation: Operation) -> ReadHandler[AsyncRetrieveResult | AsyncContentResult]:
    if operation == "retrieve":
        return HANDLER.async_vector_store_file_retrieve_handler
    return HANDLER.async_vector_store_file_content_handler


def sync_read(call: ReadCall, operation: Operation, api_base: str) -> JsonValue:
    arguments = read_arguments(call, operation, api_base)
    return JSON.validate_python(sync_handler(operation)(**arguments))


async def async_read(call: ReadCall, operation: Operation, api_base: str) -> JsonValue:
    arguments = read_arguments(call, operation, api_base)
    return JSON.validate_python(await async_handler(operation)(**arguments))


def run_sync_operations(call: ReadCall, api_base: str) -> Outcome:
    try:
        results = [sync_read(call, operation, api_base) for operation in call.operations]
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(results=results)


async def run_async_operations(call: ReadCall, api_base: str) -> Outcome:
    try:
        results = [await async_read(call, operation, api_base) for operation in call.operations]
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(results=results)


# One event loop for every read of an async call: the cached client belongs to its creating loop
def run_read(call: ReadCall, api_base: str) -> Outcome:
    if call.mode == "async":
        return asyncio.run(run_async_operations(call, api_base))
    return run_sync_operations(call, api_base)


class VectorFileReadTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_read_transport: ReadCall

    def serialize(self, api_base: str) -> str:
        outcome = run_read(self.vector_file_read_transport, api_base)
        value = ReadValue(value=outcome)
        return ReadEnvelope(vector_file_read_transport=value).model_dump_json(by_alias=True)

import asyncio
from datetime import UTC, datetime
from typing import Final, Literal

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
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["retrieve", "content"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
HANDLER: Final = BaseLLMHTTPHandler()


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


def sync_read(call: ReadCall, operation: Operation, api_base: str) -> JsonValue:
    config: BaseVectorStoreFilesConfig = OpenAIVectorStoreFilesConfig()
    params = GenericLiteLLMParams(api_base=api_base)
    headers = caller_headers(call)
    logging_obj = logging_fixture(operation)
    timeout = read_timeout(call)
    if operation == "retrieve":
        return JSON.validate_python(
            HANDLER.vector_store_file_retrieve_handler(
                vector_store_id=call.vector_store_id,
                file_id=call.file_id,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=params,
                logging_obj=logging_obj,
                extra_headers=headers,
                timeout=timeout,
            )
        )
    return JSON.validate_python(
        HANDLER.vector_store_file_content_handler(
            vector_store_id=call.vector_store_id,
            file_id=call.file_id,
            vector_store_files_provider_config=config,
            custom_llm_provider="openai",
            litellm_params=params,
            logging_obj=logging_obj,
            extra_headers=headers,
            timeout=timeout,
        )
    )


async def async_read(call: ReadCall, operation: Operation, api_base: str) -> JsonValue:
    config: BaseVectorStoreFilesConfig = OpenAIVectorStoreFilesConfig()
    params = GenericLiteLLMParams(api_base=api_base)
    headers = caller_headers(call)
    logging_obj = logging_fixture(operation)
    timeout = read_timeout(call)
    if operation == "retrieve":
        return JSON.validate_python(
            await HANDLER.async_vector_store_file_retrieve_handler(
                vector_store_id=call.vector_store_id,
                file_id=call.file_id,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=params,
                logging_obj=logging_obj,
                extra_headers=headers,
                timeout=timeout,
            )
        )
    return JSON.validate_python(
        await HANDLER.async_vector_store_file_content_handler(
            vector_store_id=call.vector_store_id,
            file_id=call.file_id,
            vector_store_files_provider_config=config,
            custom_llm_provider="openai",
            litellm_params=params,
            logging_obj=logging_obj,
            extra_headers=headers,
            timeout=timeout,
        )
    )


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


# One event loop for every read of an async call: the cached httpx client belongs to the loop that created it
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

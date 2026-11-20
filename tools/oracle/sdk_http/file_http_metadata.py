import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, cast

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.files.transformation import BaseFilesConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import OpenAIFileObject
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.file_deleted import FileDeleted
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["retrieve", "delete", "list"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
HANDLER: Final = BaseLLMHTTPHandler()


class MetadataCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    file_id: str | None = None
    purpose: str | None = None
    api_key: str | None = None
    extra_headers: dict[str, str] | None = None
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


class MetadataValue(BaseModel):
    value: Outcome


class MetadataEnvelope(BaseModel):
    file_http_metadata: MetadataValue


def logging_fixture(operation: Operation) -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=f"file_{operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="file-http-metadata",
        function_id="file-http-metadata",
    )


def execute(
    call: MetadataCall,
    config: BaseFilesConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture(call.operation)
    litellm_params: dict[str, str | None] = {"api_base": api_base, "api_key": call.api_key}
    headers: dict[str, str] = {} if call.extra_headers is None else call.extra_headers
    file_id = "" if call.file_id is None else call.file_id
    if call.operation == "retrieve" and call.mode == "sync":
        retrieved = HANDLER.retrieve_file(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4553
            file_id, config, litellm_params, headers, logging_obj, timeout=timeout
        )
        return JSON.validate_json(
            cast("OpenAIFileObject", retrieved).model_dump_json(exclude_none=True)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4563
        )
    if call.operation == "retrieve":
        fetched = asyncio.run(
            HANDLER.async_retrieve_file(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4621
                file_id, config, litellm_params, headers, logging_obj, None, timeout
            )
        )
        return JSON.validate_json(fetched.model_dump_json(exclude_none=True))
    if call.operation == "delete" and call.mode == "sync":
        removed = HANDLER.delete_file(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4677
            file_id, config, litellm_params, headers, logging_obj, timeout=timeout
        )
        return JSON.validate_json(
            cast("FileDeleted", removed).model_dump_json(exclude_none=True)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4687
        )
    if call.operation == "delete":
        deleted = asyncio.run(
            HANDLER.async_delete_file(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4745
                file_id, config, litellm_params, headers, logging_obj, None, timeout
            )
        )
        return JSON.validate_json(deleted.model_dump_json(exclude_none=True))
    if call.mode == "sync":
        listed = HANDLER.list_files(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4801
            call.purpose, config, litellm_params, headers, logging_obj, timeout=timeout
        )
        files = cast("list[OpenAIFileObject]", listed)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4811
        return [JSON.validate_json(entry.model_dump_json(exclude_none=True)) for entry in files]
    listed_async = asyncio.run(
        HANDLER.async_list_files(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L4868
            call.purpose, config, litellm_params, headers, logging_obj, None, timeout
        )
    )
    return [JSON.validate_json(entry.model_dump_json(exclude_none=True)) for entry in listed_async]


def run_metadata(call: MetadataCall, api_base: str) -> Outcome:
    config = ProviderConfigManager.get_provider_files_config(model="", provider=LlmProviders.XAI)
    if config is None:
        raise ValueError("no xAI files config")
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        result = execute(call, config, timeout, api_base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


class FileHttpMetadataRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_http_metadata: MetadataCall

    def serialize(self, api_base: str) -> str:
        outcome = run_metadata(self.file_http_metadata, api_base)
        value = MetadataValue(value=outcome)
        return MetadataEnvelope(file_http_metadata=value).model_dump_json(by_alias=True)

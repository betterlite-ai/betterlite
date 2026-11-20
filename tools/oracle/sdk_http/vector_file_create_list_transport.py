import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias

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
    VectorStoreFileChunkingStrategy as ChunkingStrategy,
)
from litellm.types.vector_store_files import (
    VectorStoreFileCreateRequest,
    VectorStoreFileListQueryParams,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["create", "list"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
# the handlers take dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

# keyword-only handlers reply with Any-carrying objects, so each signature is spelled out here
HandlerResult: TypeAlias = object
AsyncHandlerResult: TypeAlias = Coroutine[HandlerResult, HandlerResult, HandlerResult]
ExtraValues: TypeAlias = dict[str, object] | None


class Handlers(Protocol):
    def vector_store_file_create_handler(  # ast-grep-ignore: no-long-names
        self,
        *,
        vector_store_id: str,
        create_request: VectorStoreFileCreateRequest,
        vector_store_files_provider_config: BaseVectorStoreFilesConfig,  # ast-grep-ignore
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        extra_body: ExtraValues = None,
        timeout: float | None = None,
    ) -> HandlerResult: ...

    def async_vector_store_file_create_handler(  # ast-grep-ignore: no-long-names
        self,
        *,
        vector_store_id: str,
        create_request: VectorStoreFileCreateRequest,
        vector_store_files_provider_config: BaseVectorStoreFilesConfig,  # ast-grep-ignore
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        extra_body: ExtraValues = None,
        timeout: float | None = None,
    ) -> AsyncHandlerResult: ...

    def vector_store_file_list_handler(
        self,
        *,
        vector_store_id: str,
        query_params: VectorStoreFileListQueryParams,
        vector_store_files_provider_config: BaseVectorStoreFilesConfig,  # ast-grep-ignore
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        extra_query: ExtraValues = None,
        timeout: float | None = None,
    ) -> HandlerResult: ...

    def async_vector_store_file_list_handler(  # ast-grep-ignore: no-long-names
        self,
        *,
        vector_store_id: str,
        query_params: VectorStoreFileListQueryParams,
        vector_store_files_provider_config: BaseVectorStoreFilesConfig,  # ast-grep-ignore
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        extra_query: ExtraValues = None,
        timeout: float | None = None,
    ) -> AsyncHandlerResult: ...


HANDLERS: Handlers = BaseLLMHTTPHandler()


class CreateValues(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_id: str | None = None
    attributes: dict[str, str | int | float | bool] | None = None
    chunking_strategy: ChunkingStrategy | None = None


class ListValues(BaseModel):
    model_config = ConfigDict(extra="forbid")

    after: str | None = None
    before: str | None = None
    filter_status: Literal["in_progress", "completed", "failed", "cancelled"] | None = Field(
        default=None, alias="filter"
    )
    limit: int | None = None
    order: Literal["asc", "desc"] | None = None


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    vector_store_id: str
    create_request: dict[str, JsonValue] = Field(default_factory=dict)
    query_params: dict[str, JsonValue] = Field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None
    extra_query: dict[str, JsonValue] | None = None
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
    transport: TransportValue = Field(
        serialization_alias="vector_file_create_list_transport"
    )


def logging_fixture(operation: Operation) -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=f"vector_store_file_{operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="vector-file-create-list-transport",
        function_id="vector-file-create-list-transport",
    )


# the handler copies the request dict in its own order, so every key goes in as the case gave it
def requested_create(values: dict[str, JsonValue]) -> VectorStoreFileCreateRequest:
    params = CreateValues.model_validate(values)
    request: VectorStoreFileCreateRequest = {}
    for name in values:
        if name == "file_id" and params.file_id is not None:
            request["file_id"] = params.file_id
        elif name == "attributes":
            request["attributes"] = params.attributes
        elif name == "chunking_strategy":
            request["chunking_strategy"] = params.chunking_strategy
    return request


def requested_query(values: dict[str, JsonValue]) -> VectorStoreFileListQueryParams:
    params = ListValues.model_validate(values)
    query: VectorStoreFileListQueryParams = {}
    for name in values:
        if name == "after":
            query["after"] = params.after
        elif name == "before":
            query["before"] = params.before
        elif name == "filter":
            query["filter"] = params.filter_status
        elif name == "limit":
            query["limit"] = params.limit
        elif name == "order":
            query["order"] = params.order
    return query


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config: BaseVectorStoreFilesConfig = OpenAIVectorStoreFilesConfig()
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        result = execute(call, config, timeout, api_base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


def execute(
    call: TransportCall,
    config: BaseVectorStoreFilesConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture(call.operation)
    litellm_params = GenericLiteLLMParams(api_base=api_base)
    extra_body = OBJECTS.validate_python(call.extra_body)
    extra_query = OBJECTS.validate_python(call.extra_query)
    if call.operation == "create":
        create = requested_create(call.create_request)
        if call.mode == "sync":
            return JSON.validate_python(
                HANDLERS.vector_store_file_create_handler(
                    vector_store_id=call.vector_store_id,
                    create_request=create,
                    vector_store_files_provider_config=config,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    extra_headers=call.extra_headers,
                    extra_body=extra_body,
                    timeout=timeout,
                )
            )
        return JSON.validate_python(
            asyncio.run(
                HANDLERS.async_vector_store_file_create_handler(
                    vector_store_id=call.vector_store_id,
                    create_request=create,
                    vector_store_files_provider_config=config,
                    custom_llm_provider="openai",
                    litellm_params=litellm_params,
                    logging_obj=logging_obj,
                    extra_headers=call.extra_headers,
                    extra_body=extra_body,
                    timeout=timeout,
                )
            )
        )
    query = requested_query(call.query_params)
    if call.mode == "sync":
        return JSON.validate_python(
            HANDLERS.vector_store_file_list_handler(
                vector_store_id=call.vector_store_id,
                query_params=query,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=litellm_params,
                logging_obj=logging_obj,
                extra_headers=call.extra_headers,
                extra_query=extra_query,
                timeout=timeout,
            )
        )
    return JSON.validate_python(
        asyncio.run(
            HANDLERS.async_vector_store_file_list_handler(
                vector_store_id=call.vector_store_id,
                query_params=query,
                vector_store_files_provider_config=config,
                custom_llm_provider="openai",
                litellm_params=litellm_params,
                logging_obj=logging_obj,
                extra_headers=call.extra_headers,
                extra_query=extra_query,
                timeout=timeout,
            )
        )
    )


class FileCreateListTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_create_list_transport: TransportCall = Field(
        alias="vector_file_create_list_transport"
    )

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.file_create_list_transport, api_base)
        envelope = TransportEnvelope(transport=TransportValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)

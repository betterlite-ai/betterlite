import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.vector_store.transformation import BaseVectorStoreConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.vector_stores.transformation import OpenAIVectorStoreConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import (
    VectorStoreChunkingStrategy,
    VectorStoreCreateResponse,
    VectorStoreExpirationPolicy,
)
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams as VectorStoreParams,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["update", "delete"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

UpdateWork = VectorStoreCreateResponse | Coroutine[object, object, VectorStoreCreateResponse]
DeleteWork = JsonValue | Coroutine[object, object, JsonValue]

UpdateCall = Callable[
    [
        str,
        VectorStoreParams,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    UpdateWork,
]

AsyncUpdateCall = Callable[
    [
        str,
        VectorStoreParams,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, VectorStoreCreateResponse],
]

DeleteCall = Callable[
    [
        str,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    DeleteWork,
]

AsyncDeleteCall = Callable[
    [
        str,
        BaseVectorStoreConfig,
        str,
        GenericLiteLLMParams,
        Logging,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, JsonValue],
]


class OptionalParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str | None = None
    file_ids: list[str] | None = None
    expires_after: VectorStoreExpirationPolicy | None = None
    chunking_strategy: VectorStoreChunkingStrategy | None = None
    metadata: dict[str, str] | None = None


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    vector_store_id: str
    optional_params: OptionalParams = Field(default_factory=OptionalParams)
    extra_headers: dict[str, object] | None = None  # ast-grep-ignore: no-loose-types
    extra_body: dict[str, object] | None = None  # ast-grep-ignore: no-loose-types
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
    vector_store_transport: TransportValue


class Handlers(Protocol):
    @property
    def vector_store_update_handler(self) -> UpdateCall: ...

    @property
    def async_vector_store_update_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncUpdateCall: ...

    @property
    def vector_store_delete_handler(self) -> DeleteCall: ...

    @property
    def async_vector_store_delete_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncDeleteCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


def logging_fixture(operation: Operation) -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type=f"vector_store_{operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="vector-store-transport",
        function_id="vector-store-transport",
    )


def requested_params(optional: OptionalParams) -> VectorStoreParams:
    params: VectorStoreParams = {}
    if "name" in optional.model_fields_set:
        params["name"] = optional.name
    if "file_ids" in optional.model_fields_set:
        params["file_ids"] = optional.file_ids
    if "expires_after" in optional.model_fields_set:
        params["expires_after"] = optional.expires_after
    if "chunking_strategy" in optional.model_fields_set:
        params["chunking_strategy"] = optional.chunking_strategy
    if "metadata" in optional.model_fields_set:
        params["metadata"] = optional.metadata
    return params


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config: BaseVectorStoreConfig = OpenAIVectorStoreConfig()
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    try:
        result = execute(call, config, timeout, api_base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


def execute(
    call: TransportCall,
    config: BaseVectorStoreConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture(call.operation)
    litellm_params = GenericLiteLLMParams(api_base=api_base)
    if call.operation == "update":
        return update(call, config, logging_obj, litellm_params, timeout)
    return delete(call, config, logging_obj, litellm_params, timeout)


def update(
    call: TransportCall,
    config: BaseVectorStoreConfig,
    logging_obj: Logging,
    litellm_params: GenericLiteLLMParams,
    timeout: float | None,
) -> JsonValue:
    optional = requested_params(call.optional_params)
    if call.mode == "sync":
        reply = HANDLER.vector_store_update_handler(
            call.vector_store_id,
            optional,
            config,
            "openai",
            litellm_params,
            logging_obj,
            call.extra_headers,
            call.extra_body,
            timeout,
        )
    else:
        pending = HANDLER.async_vector_store_update_handler(
            call.vector_store_id,
            optional,
            config,
            "openai",
            litellm_params,
            logging_obj,
            call.extra_headers,
            call.extra_body,
            timeout,
        )
        reply = asyncio.run(pending)
    return JSON.validate_python(reply)


def delete(
    call: TransportCall,
    config: BaseVectorStoreConfig,
    logging_obj: Logging,
    litellm_params: GenericLiteLLMParams,
    timeout: float | None,
) -> JsonValue:
    if call.mode == "sync":
        reply = HANDLER.vector_store_delete_handler(
            call.vector_store_id,
            config,
            "openai",
            litellm_params,
            logging_obj,
            call.extra_headers,
            call.extra_body,
            timeout,
        )
    else:
        pending = HANDLER.async_vector_store_delete_handler(
            call.vector_store_id,
            config,
            "openai",
            litellm_params,
            logging_obj,
            call.extra_headers,
            call.extra_body,
            timeout,
        )
        reply = asyncio.run(pending)
    return JSON.validate_python(reply)


class VectorStoreTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.vector_store_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(vector_store_transport=value).model_dump_json(by_alias=True)

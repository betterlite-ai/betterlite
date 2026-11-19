import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.responses.main import DeleteResponseResult
from litellm.types.router import GenericLiteLLMParams
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Operation = Literal["delete", "cancel"]
Mode = Literal["sync", "async"]

OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
DELETE_RESULT: Final = TypeAdapter(DeleteResponseResult)
RESPONSE: Final = TypeAdapter(ResponsesAPIResponse)

Overrides = dict[str, object] | None
DeleteWork = DeleteResponseResult | Coroutine[object, object, DeleteResponseResult]
CancelWork = ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]

SyncDelete = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        Overrides,
        Overrides,
        float | httpx.Timeout | None,
    ],
    DeleteWork,
]
AsyncDelete = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        Overrides,
        Overrides,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, DeleteResponseResult],
]
SyncCancel = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        Overrides,
        Overrides,
        float | httpx.Timeout | None,
    ],
    CancelWork,
]
AsyncCancel = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        Overrides,
        Overrides,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, ResponsesAPIResponse],
]


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    response_id: str
    api_key: str | None = None
    extra_headers: dict[str, JsonValue] | None = None
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
    responses_write_transport: TransportValue


class HiddenParams(TypedDict):
    headers: dict[str, JsonValue]
    additional_headers: dict[str, JsonValue]


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    params: HiddenParams = Field(alias="_hidden_params")


class Handlers(Protocol):
    @property
    def delete_response_api_handler(self) -> SyncDelete: ...

    @property
    def async_delete_response_api_handler(self) -> AsyncDelete: ...

    @property
    def cancel_response_api_handler(self) -> SyncCancel: ...

    @property
    def async_cancel_response_api_handler(self) -> AsyncCancel: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="responses-write-transport",
        function_id="responses-write-transport",
    )


def deleted_value(deleted: DeleteResponseResult) -> JsonValue:
    fields: dict[str, JsonValue] = {
        "id": deleted.id,
        "object": deleted.object,
        "deleted": deleted.deleted,
    }
    return fields


def cancelled_value(reply: ResponsesAPIResponse) -> JsonValue:
    hidden = Hidden.model_validate(reply)
    fields: dict[str, JsonValue] = {
        "id": reply.id,
        "created_at": reply.created_at,
        "object": reply.object,
        "status": reply.status,
        "model": reply.model,
        "headers": hidden.params["headers"],
        "additional_headers": hidden.params["additional_headers"],
    }
    return fields


def call_operation(
    call: TransportCall,
    config: BaseResponsesAPIConfig,
    litellm_params: GenericLiteLLMParams,
    timeout: float | httpx.Timeout | None,
) -> JsonValue:
    logging_obj = logging_fixture()
    extra_headers = OBJECTS.validate_python(call.extra_headers)
    extra_body = OBJECTS.validate_python(call.extra_body)
    if call.operation == "delete":
        if call.mode == "sync":
            deleted_work = HANDLER.delete_response_api_handler(
                call.response_id,
                config,
                litellm_params,
                logging_obj,
                "openai",
                extra_headers,
                extra_body,
                timeout,
            )
            return deleted_value(DELETE_RESULT.validate_python(deleted_work))
        deleted_answer = asyncio.run(
            HANDLER.async_delete_response_api_handler(
                call.response_id,
                config,
                litellm_params,
                logging_obj,
                "openai",
                extra_headers,
                extra_body,
                timeout,
            )
        )
        return deleted_value(deleted_answer)
    if call.mode == "sync":
        cancelled_work = HANDLER.cancel_response_api_handler(
            call.response_id,
            config,
            litellm_params,
            logging_obj,
            "openai",
            extra_headers,
            extra_body,
            timeout,
        )
        return cancelled_value(RESPONSE.validate_python(cancelled_work))
    cancelled_answer = asyncio.run(
        HANDLER.async_cancel_response_api_handler(
            call.response_id,
            config,
            litellm_params,
            logging_obj,
            "openai",
            extra_headers,
            extra_body,
            timeout,
        )
    )
    return cancelled_value(cancelled_answer)


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    # LiteLLM's responses config __init__ is untyped and stateless
    config: BaseResponsesAPIConfig = OpenAIResponsesAPIConfig.__new__(OpenAIResponsesAPIConfig)
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    litellm_params = GenericLiteLLMParams(api_base=api_base, api_key=call.api_key)
    try:
        result = call_operation(call, config, litellm_params, timeout)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


class ResponsesWriteTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_write_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.responses_write_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(responses_write_transport=value).model_dump_json(by_alias=True)

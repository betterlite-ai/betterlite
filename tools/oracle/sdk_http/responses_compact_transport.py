import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
INPUT: TypeAdapter[str | ResponseInputParam] = TypeAdapter(str | ResponseInputParam)
# the handler takes dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object])
NULLABLE_OBJECTS: Final = TypeAdapter(dict[str, object] | None)
RESPONSES_API: Final[TypeAdapter[ResponsesAPIResponse]] = TypeAdapter(ResponsesAPIResponse)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

CompactWork = ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]
AsyncCompactWork = Coroutine[object, object, ResponsesAPIResponse]

CompactCall = Callable[
    [
        str,
        str | ResponseInputParam,
        BaseResponsesAPIConfig,
        dict[str, object],
        GenericLiteLLMParams,
        Logging,
        str | None,
        dict[str, object] | None,
        dict[str, object] | None,
        float | None,
    ],
    CompactWork,
]
AsyncCompactCall = Callable[
    [
        str,
        str | ResponseInputParam,
        BaseResponsesAPIConfig,
        dict[str, object],
        GenericLiteLLMParams,
        Logging,
        str | None,
        dict[str, object] | None,
        dict[str, object] | None,
        float | None,
    ],
    AsyncCompactWork,
]


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    model: str
    input: JsonValue
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: int | None = None
    api_query: str | None = None


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
    responses_compact_transport: TransportValue


class Handlers(Protocol):
    @property
    def compact_response_api_handler(self) -> CompactCall: ...

    @property
    def async_compact_response_api_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncCompactCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


def logging_fixture() -> Logging:
    return Logging(
        model="gpt-4o",
        messages="",
        stream=False,
        call_type="compact_response",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="compact-transport",
        function_id="compact-transport",
    )


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config = ProviderConfigManager.get_provider_responses_api_config(
        provider="openai", model=call.model
    )
    if config is None:
        raise ValueError("the openai responses config is registered")
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    base = api_base if call.api_query is None else f"{api_base}?{call.api_query}"
    try:
        result = execute(call, config, timeout, base)
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return Outcome(result=result)


def execute(
    call: TransportCall,
    config: BaseResponsesAPIConfig,
    timeout: float | None,
    api_base: str,
) -> JsonValue:
    logging_obj = logging_fixture()
    litellm_params = GenericLiteLLMParams(api_base=api_base)
    optional_params = OBJECTS.validate_python(call.optional_params)
    extra_headers = NULLABLE_OBJECTS.validate_python(call.extra_headers)
    extra_body = NULLABLE_OBJECTS.validate_python(call.extra_body)
    if call.mode == "sync":
        work = HANDLER.compact_response_api_handler(
            call.model,
            INPUT.validate_python(call.input),
            config,
            optional_params,
            litellm_params,
            logging_obj,
            "openai",
            extra_headers,
            extra_body,
            timeout,
        )
    else:
        work = asyncio.run(
            HANDLER.async_compact_response_api_handler(
                call.model,
                INPUT.validate_python(call.input),
                config,
                optional_params,
                litellm_params,
                logging_obj,
                "openai",
                extra_headers,
                extra_body,
                timeout,
            )
        )
    response = RESPONSES_API.validate_python(work)
    return JSON.validate_json(response.model_dump_json())


class CompactTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_compact_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.responses_compact_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(responses_compact_transport=value).model_dump_json(by_alias=True)

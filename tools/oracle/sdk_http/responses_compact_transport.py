import asyncio
from datetime import UTC, datetime
from typing import Final, Literal

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
INPUT: TypeAdapter[str | ResponseInputParam] = TypeAdapter(str | ResponseInputParam)
# the handler takes dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


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


HANDLER: BaseLLMHTTPHandler = BaseLLMHTTPHandler()


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
    config: BaseResponsesAPIConfig = OpenAIResponsesAPIConfig()
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
    optional = OBJECTS.validate_python(call.optional_params)
    optional_params: dict[str, object] = {} if optional is None else optional
    extra_headers = OBJECTS.validate_python(call.extra_headers)
    extra_body = OBJECTS.validate_python(call.extra_body)
    if call.mode == "async":
        result = asyncio.run(
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
    else:
        result = HANDLER.compact_response_api_handler(
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
    response = ResponsesAPIResponse.model_validate(result)
    return JSON.validate_json(response.model_dump_json())


class CompactTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_compact_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.responses_compact_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(responses_compact_transport=value).model_dump_json(by_alias=True)

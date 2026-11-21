import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

import litellm
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.anthropic_messages.transformation import BaseAnthropicMessagesConfig
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.responses.streaming_iterator import BaseResponsesAPIStreamingIterator as StreamingBase
from litellm.responses.streaming_iterator import MockResponsesAPIStreamingIterator as MockSynthetic
from litellm.types.llms.openai import ResponseCompletedEvent, ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import CallTypes
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from responses_compact_transport import (
    INPUT,
    JSON,
    NULLABLE_OBJECTS,
    OBJECTS,
    PROVIDER_ERROR,
    RESPONSES_API,
    Failure,
    Mode,
)

StreamKind = Literal["live", "synthetic"]

JSON_MAP: Final = TypeAdapter(dict[str, JsonValue])
MESSAGES: Final = TypeAdapter(list[JsonValue])
NULLABLE_JSON: Final = TypeAdapter(dict[str, JsonValue] | None)
COMPLETED: Final[TypeAdapter[ResponseCompletedEvent]] = TypeAdapter(ResponseCompletedEvent)
LITELLM_PARAMS: Final[TypeAdapter[GenericLiteLLMParams]] = TypeAdapter(GenericLiteLLMParams)

CreateWork = (
    ResponsesAPIResponse
    | StreamingBase
    | Coroutine[object, object, ResponsesAPIResponse | StreamingBase]
)

SyncCreateCall = Callable[[
    str, str | ResponseInputParam, BaseResponsesAPIConfig, dict[str, object], str,
    GenericLiteLLMParams, Logging, dict[str, object] | None, dict[str, object] | None,
    float | None, HTTPHandler | AsyncHTTPHandler | None, bool, bool, dict[str, object] | None,
], CreateWork]
AsyncCreateCall = Callable[[
    str, str | ResponseInputParam, BaseResponsesAPIConfig, dict[str, object], str,
    GenericLiteLLMParams, Logging, dict[str, object] | None, dict[str, object] | None,
    float | None, HTTPHandler | AsyncHTTPHandler | None, bool, dict[str, object] | None,
], Coroutine[object, object, ResponsesAPIResponse | StreamingBase]]


class Handlers(Protocol):
    @property
    def response_api_handler(self) -> SyncCreateCall: ...

    @property
    def async_response_api_handler(self) -> AsyncCreateCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class TransportCall(StrictModel):
    mode: Mode
    model: str
    input: JsonValue
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: float | None = None
    fake_stream: JsonValue = None
    litellm_metadata: dict[str, JsonValue] | None = None
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)
    pre_hook: dict[str, JsonValue] | None = None
    agentic: bool = False
    agentic_result: JsonValue = None


class SetupView(StrictModel):
    model: str
    custom_llm_provider: str | None = None
    litellm_metadata: dict[str, JsonValue] | None = None
    request_data: dict[str, JsonValue] | None = None
    call_type: str | None = None


class StreamView(StrictModel):
    kind: StreamKind
    setup: SetupView
    response: JsonValue = None


class Outcome(BaseModel):
    result: JsonValue = None
    failure: Failure | None = None
    stream: StreamView | None = None


class CreateTransportValue(BaseModel):
    value: Outcome


class CreateTransportEnvelope(BaseModel):
    responses_create_transport: CreateTransportValue


# The sync handler's _run_sync_responses_pre_call_deployment_hook, over the case's patch
class PatchHook(CustomLogger):
    def __init__(self, patch: dict[str, JsonValue]) -> None:
        self.patch = patch

    @override
    async def async_pre_call_deployment_hook(
        self,
        # ast-grep-ignore: no-loose-types
        kwargs: dict[str, object],
        call_type: CallTypes | None,
    ) -> dict[str, JsonValue] | None:
        merged = JSON_MAP.validate_python(kwargs)
        merged.update(self.patch)
        return merged


# LiteLLM's messages argument: a list input as it is, text as one user turn.
def followup_messages(value: str | ResponseInputParam) -> list[JsonValue]:
    match value:
        case str():
            return [{"role": "user", "content": value}]
        case list():
            return MESSAGES.validate_python(value)


# The scripted async_should_run_agentic_loop and async_run_agentic_loop pair
class ScriptedAgentic(CustomLogger):
    def __init__(self, result: JsonValue, messages: list[JsonValue]) -> None:
        self.result = result
        self.messages = messages

    def check_messages(self, messages: list[dict[str, JsonValue]]) -> None:
        found: list[JsonValue] = [*messages]
        if found != self.messages:
            raise ValueError(f"the agentic messages are {found!r}, not {self.messages!r}")

    @override
    async def async_should_run_agentic_loop(
        self,
        # ast-grep-ignore: no-loose-types
        response: object,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[dict[str, JsonValue]] | None,
        stream: bool,
        custom_llm_provider: str,
        kwargs: dict[str, JsonValue],
    ) -> tuple[bool, dict[str, JsonValue]]:
        self.check_messages(messages)
        return True, {}

    @override
    async def async_run_agentic_loop(
        self,
        tools: dict[str, JsonValue],
        model: str,
        messages: list[dict[str, JsonValue]],
        # ast-grep-ignore: no-loose-types
        response: object,
        # ast-grep-ignore: no-long-names
        anthropic_messages_provider_config: BaseAnthropicMessagesConfig | None,
        # ast-grep-ignore: no-long-names
        anthropic_messages_optional_request_params: dict[str, JsonValue],
        logging_obj: Logging,
        stream: bool,
        kwargs: dict[str, JsonValue],
    ) -> ResponsesAPIResponse | None:
        self.check_messages(messages)
        if self.result is None:
            return None
        return RESPONSES_API.validate_python(self.result)


def install(call: TransportCall) -> None:
    callbacks: list[CustomLogger] = []
    if call.pre_hook is not None:
        callbacks.append(PatchHook(call.pre_hook))
    if call.agentic:
        messages = followup_messages(INPUT.validate_python(call.input))
        callbacks.append(ScriptedAgentic(call.agentic_result, messages))
    litellm.callbacks = [*callbacks]


def logging_fixture(model: str) -> Logging:
    return Logging(
        model=model,
        messages="",
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="create-transport",
        function_id="create-transport",
    )


def invoke(
    call: TransportCall,
    config: BaseResponsesAPIConfig,
    litellm_params: GenericLiteLLMParams,
    logging_obj: Logging,
) -> CreateWork:
    optional_params = OBJECTS.validate_python(call.optional_params)
    extra_headers = NULLABLE_OBJECTS.validate_python(call.extra_headers)
    extra_body = NULLABLE_OBJECTS.validate_python(call.extra_body)
    metadata = NULLABLE_OBJECTS.validate_python(call.litellm_metadata)
    timeout = None if call.timeout_seconds is None else float(call.timeout_seconds)
    fake_stream = call.fake_stream is True
    is_async = False
    response_input = INPUT.validate_python(call.input)
    if call.mode == "sync":
        return HANDLER.response_api_handler(
            call.model, response_input, config, optional_params, "openai", litellm_params,
            logging_obj, extra_headers, extra_body, timeout, None, is_async, fake_stream, metadata,
        )
    return asyncio.run(
        HANDLER.async_response_api_handler(
            call.model, response_input, config, optional_params, "openai", litellm_params,
            logging_obj, extra_headers, extra_body, timeout, None, fake_stream, metadata,
        )
    )


def synthetic_response(iterator: StreamingBase) -> JsonValue:
    event = COMPLETED.validate_python(iterator.completed_response)
    return JSON.validate_json(event.response.model_dump_json())


def stream_view(kind: StreamKind, iterator: StreamingBase) -> StreamView:
    request_data: dict[str, JsonValue] | None = JSON_MAP.validate_python(iterator.request_data)
    if not request_data:
        request_data = None
    return StreamView(
        kind=kind,
        setup=SetupView(
            model=iterator.model,
            custom_llm_provider=iterator.custom_llm_provider,
            litellm_metadata=NULLABLE_JSON.validate_python(iterator.litellm_metadata),
            request_data=request_data,
            call_type=iterator.call_type,
        ),
        response=None if kind == "live" else synthetic_response(iterator),
    )


def observed(work: CreateWork) -> Outcome:
    match work:
        case MockSynthetic():
            return Outcome(stream=stream_view("synthetic", work))
        case StreamingBase():
            return Outcome(stream=stream_view("live", work))
        case object():
            response = RESPONSES_API.validate_python(work)
            return Outcome(result=JSON.validate_json(response.model_dump_json()))


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    config = ProviderConfigManager.get_provider_responses_api_config(
        provider="openai", model=call.model
    )
    if config is None:
        raise ValueError("the openai responses config is registered")
    install(call)
    litellm_params = LITELLM_PARAMS.validate_python({**call.litellm_params, "api_base": api_base})
    try:
        work = invoke(call, config, litellm_params, logging_fixture(call.model))
    except PROVIDER_ERROR as error:
        return Outcome(failure=Failure.model_validate(error))
    return observed(work)


class CreateTransportRequest(StrictModel):
    responses_create_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        value = CreateTransportValue(value=run_transport(self.responses_create_transport, api_base))
        return CreateTransportEnvelope(responses_create_transport=value).model_dump_json(
            by_alias=True
        )

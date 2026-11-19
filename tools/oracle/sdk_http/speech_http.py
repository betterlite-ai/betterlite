import asyncio
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.text_to_speech.transformation import (
    BaseTextToSpeechConfig,
    TextToSpeechRequestData,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from litellm.types.utils import CallTypes
from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import override

MODEL: Final = "speech-model"
PROVIDER: Final = "openai"
DEFAULT_TIMEOUT: Final = 600.0
API_BASE_REQUIRED: Final = "api_base is required"

Mode = Literal["sync", "async"]


class Transformed(BaseModel):
    model_config = ConfigDict(extra="forbid")

    dict_body: dict[str, JsonValue] | None = None
    ssml_body: str | None = None
    text_body: str | None = None
    binary_body: str | None = None
    headers: dict[str, str] | None = None


class Step(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Mode
    input: str
    voice: str | None = None
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    api_key: str | None = None
    ssl_verify: bool | None = None
    extra_headers: dict[str, str] | None = None
    validated: dict[str, str] | None = None
    transformed: Transformed
    timeout_seconds: int | None = None
    supplied: Mode | None = None


class SpeechHttpCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[Step]


class BinaryValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    status: int
    body: list[int]
    headers: dict[str, str]


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    class_name: str = Field(serialization_alias="class")
    status: int
    synthesized: bool
    message: str
    headers: dict[str, str]


class StepOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: BinaryValue | None = None
    failure: Failure | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    steps: list[StepOutcome]


class SpeechHttpValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class SpeechHttpOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_http: SpeechHttpValue


SpeechWork = (
    HttpxBinaryResponseContent
    | Coroutine[object, object, HttpxBinaryResponseContent]
)
Client = HTTPHandler | AsyncHTTPHandler | None

SyncCall = Callable[
    [
        str,
        str,
        str | None,
        BaseTextToSpeechConfig,
        dict[str, JsonValue],
        str,
        dict[str, JsonValue],
        Logging,
        float | httpx.Timeout,
        dict[str, object] | None,
        Client,
    ],
    SpeechWork,
]

AsyncCall = Callable[
    [
        str,
        str,
        str | None,
        BaseTextToSpeechConfig,
        dict[str, JsonValue],
        str,
        dict[str, JsonValue],
        Logging,
        float | httpx.Timeout,
        dict[str, object] | None,
        Client,
    ],
    Coroutine[object, object, HttpxBinaryResponseContent],
]


class Handlers(Protocol):
    @property
    def text_to_speech_handler(self) -> SyncCall: ...

    @property
    def async_text_to_speech_handler(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


class StepConfig(BaseTextToSpeechConfig):
    step: Step
    error_headers: dict[str, str]

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self, *args: JsonValue, **kwargs: JsonValue
    ) -> tuple[str | None, dict[str, JsonValue]]:
        return None, {}

    @override
    def validate_environment(
        self,
        headers: dict[str, JsonValue],
        model: str,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]:
        return {} if self.step.validated is None else dict(self.step.validated)

    @override
    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str:
        if api_base is None:
            raise ValueError(API_BASE_REQUIRED)
        return api_base

    @override
    def transform_text_to_speech_request(  # ast-grep-ignore: no-long-names
        self, *args: JsonValue, **kwargs: JsonValue
    ) -> TextToSpeechRequestData:
        return request_data(self.step.transformed)

    @override
    def transform_text_to_speech_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> HttpxBinaryResponseContent:
        return HttpxBinaryResponseContent(raw_response)

    @override
    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException:
        self.error_headers = dict(headers)
        return BaseLLMException(
            status_code=status_code, message=error_message, headers=headers
        )


def step_config(step: Step) -> StepConfig:
    config = StepConfig()
    config.step = step
    config.error_headers = {}
    return config


# text_body and binary_body have no send branch, so the typed request data drops them like the Rust side does
def request_data(transformed: Transformed) -> TextToSpeechRequestData:
    data: TextToSpeechRequestData = {}
    if transformed.dict_body is not None:
        data["dict_body"] = transformed.dict_body
    if transformed.ssml_body is not None:
        data["ssml_body"] = transformed.ssml_body
    if transformed.headers is not None:
        data["headers"] = transformed.headers
    return data


def litellm_params(step: Step, api_base: str) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"api_base": api_base}
    if step.api_key is not None:
        params["api_key"] = step.api_key
    if step.ssl_verify is not None:
        params["ssl_verify"] = step.ssl_verify
    return params


def provided_headers(headers: dict[str, str] | None) -> dict[str, object] | None:
    if headers is None:
        return None
    mapped: dict[str, object] = {}
    for name, value in headers.items():
        mapped[name] = value
    return mapped


def supplied_client(kind: Mode | None) -> Client:
    match kind:
        case None:
            return None
        case "sync":
            return HTTPHandler()
        case "async":
            return AsyncHTTPHandler()


async def closed(client: Client) -> None:
    match client:
        case HTTPHandler():
            client.close()
        case AsyncHTTPHandler():
            await client.close()
        case None:
            pass


def timeout_of(step: Step) -> float:
    return DEFAULT_TIMEOUT if step.timeout_seconds is None else float(step.timeout_seconds)


def logging_object() -> Logging:
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=CallTypes.speech,
        start_time=datetime.now(UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


async def settled(work: SpeechWork) -> HttpxBinaryResponseContent:
    match work:
        case HttpxBinaryResponseContent():
            return work
        case Coroutine():
            return await work


def binary(response: HttpxBinaryResponseContent) -> BinaryValue:
    raw = response.response
    return BinaryValue(
        status=raw.status_code,
        body=list(raw.content),
        headers=dict(raw.headers),
    )


def failure(error: BaseLLMException, headers: dict[str, str]) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=error.status_code,
        synthesized=error.status_code_is_synthesized,
        message=error.message,
        headers=headers,
    )


async def execute_step(step: Step, api_base: str) -> StepOutcome:
    config = step_config(step)
    params = litellm_params(step, api_base)
    extra = provided_headers(step.extra_headers)
    client = supplied_client(step.supplied)
    try:
        match step.mode:
            case "sync":
                work = HANDLER.text_to_speech_handler(
                    MODEL,
                    step.input,
                    step.voice,
                    config,
                    step.optional_params,
                    PROVIDER,
                    params,
                    logging_object(),
                    timeout_of(step),
                    extra,
                    client,
                )
                response = await settled(work)
            case "async":
                response = await HANDLER.async_text_to_speech_handler(
                    MODEL,
                    step.input,
                    step.voice,
                    config,
                    step.optional_params,
                    PROVIDER,
                    params,
                    logging_object(),
                    timeout_of(step),
                    extra,
                    client,
                )
    except BaseLLMException as error:
        return StepOutcome(failure=failure(error, config.error_headers))
    finally:
        await closed(client)
    return StepOutcome(value=binary(response))


async def collect_steps(steps: list[Step], api_base: str) -> Outcome:
    outcomes: list[StepOutcome] = []
    for step in steps:
        outcomes.append(await execute_step(step, api_base))
    return Outcome(steps=outcomes)


def run_steps(call: SpeechHttpCall, api_base: str) -> Outcome:
    return asyncio.run(collect_steps(call.steps, api_base))


class SpeechHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_http: SpeechHttpCall

    def serialize(self, api_base: str) -> str:
        outcome = run_steps(self.speech_http, api_base)
        envelope = SpeechHttpOutcome(speech_http=SpeechHttpValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)

import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal

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
Client = HTTPHandler | AsyncHTTPHandler | None
SpeechWork = HttpxBinaryResponseContent | Coroutine[object, object, HttpxBinaryResponseContent]


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
    status: int
    body: list[int]
    headers: dict[str, str]


class Failure(BaseModel):
    class_name: str = Field(serialization_alias="class")
    status: int
    synthesized: bool
    message: str
    headers: dict[str, str]


class StepOutcome(BaseModel):
    value: BinaryValue | None = None
    failure: Failure | None = None


class Outcome(BaseModel):
    steps: list[StepOutcome]


class SpeechHttpValue(BaseModel):
    value: Outcome


class SpeechHttpOutcome(BaseModel):
    speech_http: SpeechHttpValue


HANDLER: Final = BaseLLMHTTPHandler()


class StepConfig(BaseTextToSpeechConfig):
    step: Step
    error_headers: dict[str, str]

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return []

    @override
    def map_openai_params(
        self,
        model: str,
        optional_params: dict[str, JsonValue],
        voice: str | dict[str, JsonValue] | None = None,
        drop_params: bool = False,
        kwargs: dict[str, JsonValue] | None = None,
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
        self,
        model: str,
        api_base: str | None,
        litellm_params: dict[str, JsonValue],
    ) -> str:
        if api_base is None:
            raise ValueError(API_BASE_REQUIRED)
        return api_base

    # the handler passes input= by keyword and input shadows a builtin, so this one absorbs it
    @override
    def transform_text_to_speech_request(  # ast-grep-ignore: no-long-names
        self, *_args: JsonValue, **_kwargs: JsonValue
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
            status_code=status_code, message=error_message, headers=self.error_headers
        )


def step_config(step: Step) -> StepConfig:
    config = StepConfig()
    config.step = step
    config.error_headers = {}
    return config


# text_body and binary_body have no send branch, so both sides drop them
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


# ast-grep-ignore: no-loose-types - the handler takes extra_headers as dict[str, object]
def provided_headers(headers: dict[str, str] | None) -> dict[str, object] | None:
    if headers is None:
        return None
    mapped: dict[str, object] = {}  # ast-grep-ignore: no-loose-types
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


def logging_object() -> Logging:
    now = datetime.now(UTC)
    return Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type=CallTypes.speech,
        start_time=now,
        litellm_call_id="call-1",
        function_id="fn-1",
        litellm_trace_id="trace-1",
    )


async def settled(work: SpeechWork) -> HttpxBinaryResponseContent:
    match work:
        case Coroutine():
            return await work
        case object():
            return work


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
    timeout = DEFAULT_TIMEOUT if step.timeout_seconds is None else float(step.timeout_seconds)
    try:
        match step.mode:
            case "sync":
                work = HANDLER.text_to_speech_handler(
                    MODEL, step.input, step.voice, config, step.optional_params,
                    PROVIDER, params, logging_object(), timeout, extra, client,
                )
                response = await settled(work)
            case "async":
                response = await HANDLER.async_text_to_speech_handler(
                    MODEL, step.input, step.voice, config, step.optional_params,
                    PROVIDER, params, logging_object(), timeout, extra, client,
                )
    except BaseLLMException as error:
        return StepOutcome(failure=failure(error, config.error_headers))
    finally:
        await closed(client)
    return StepOutcome(value=binary(response))


async def collect_steps(steps: list[Step], api_base: str) -> Outcome:
    return Outcome(steps=[await execute_step(step, api_base) for step in steps])


class SpeechHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_http: SpeechHttpCall

    def serialize(self, api_base: str) -> str:
        outcome = asyncio.run(collect_steps(self.speech_http.steps, api_base))
        envelope = SpeechHttpOutcome(speech_http=SpeechHttpValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)

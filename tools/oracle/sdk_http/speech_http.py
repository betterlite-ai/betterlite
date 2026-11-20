import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.text_to_speech.transformation import BaseTextToSpeechConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from litellm.types.utils import CallTypes
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

MODEL: Final = "speech-model"
PROVIDER: Final = "openai"
DEFAULT_TIMEOUT: Final = 600.0
API_BASE_REQUIRED: Final = "api_base is required"

Mode = Literal["sync", "async"]
Client = HTTPHandler | AsyncHTTPHandler | None
SpeechWork = HttpxBinaryResponseContent | Coroutine[object, object, HttpxBinaryResponseContent]
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


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


class RequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]
    ssml_body: str
    headers: dict[str, str]


class BinaryValue(BaseModel):
    status: int
    body: list[int]
    headers: dict[str, str]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    class_name: str = Field(default="BaseLLMException", serialization_alias="class")
    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
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
    def __init__(self, step: Step) -> None:
        self.step = step
        self.data: RequestData = {}
        if step.transformed.dict_body is not None:
            self.data["dict_body"] = step.transformed.dict_body
        if step.transformed.ssml_body is not None:
            self.data["ssml_body"] = step.transformed.ssml_body
        if step.transformed.headers is not None:
            self.data["headers"] = step.transformed.headers

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
    ) -> RequestData:
        return self.data

    @override
    def transform_text_to_speech_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> HttpxBinaryResponseContent:
        return HttpxBinaryResponseContent(raw_response)

    @override
    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException:
        return BaseLLMException(
            status_code=status_code, message=error_message, headers=headers
        )


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
        case Coroutine():
            return await work
        case object():
            return work


async def execute_step(step: Step, api_base: str) -> StepOutcome:
    config = StepConfig(step)
    params: dict[str, JsonValue] = {"api_base": api_base}
    if step.api_key is not None:
        params["api_key"] = step.api_key
    if step.ssl_verify is not None:
        params["ssl_verify"] = step.ssl_verify
    client: Client = None
    if step.supplied == "sync":
        client = HTTPHandler()
    if step.supplied == "async":
        client = AsyncHTTPHandler()
    extra = OBJECTS.validate_python(step.extra_headers)
    timeout = DEFAULT_TIMEOUT if step.timeout_seconds is None else float(step.timeout_seconds)
    try:
        if step.mode == "sync":
            work = HANDLER.text_to_speech_handler(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11827-L11841
                MODEL,
                step.input,
                step.voice,
                config,
                step.optional_params,
                PROVIDER,
                params,
                logging_object(),
                timeout,
                extra,
                client,
            )
            response = await settled(work)
        else:
            response = await HANDLER.async_text_to_speech_handler(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11944-L11955
                MODEL,
                step.input,
                step.voice,
                config,
                step.optional_params,
                PROVIDER,
                params,
                logging_object(),
                timeout,
                extra,
                client,
            )
    except PROVIDER_ERROR as error:
        return StepOutcome(failure=Failure.model_validate(error))
    raw = response.response
    return StepOutcome(
        value=BinaryValue(
            status=raw.status_code,
            body=list(raw.content),
            headers=dict(raw.headers),
        )
    )


async def collect_steps(steps: list[Step], api_base: str) -> Outcome:
    outcomes = [await execute_step(step, api_base) for step in steps]
    return Outcome(steps=outcomes)


class SpeechHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_http: SpeechHttpCall

    def serialize(self, api_base: str) -> str:
        outcome = asyncio.run(collect_steps(self.speech_http.steps, api_base))
        value = SpeechHttpValue(value=outcome)
        return SpeechHttpOutcome(speech_http=value).model_dump_json(by_alias=True)

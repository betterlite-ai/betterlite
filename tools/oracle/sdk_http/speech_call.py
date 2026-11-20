import os
from collections.abc import Coroutine
from concurrent.futures import ThreadPoolExecutor
from typing import Final, TypeAlias

import litellm
import openai
from litellm.main import speech as litellm_speech
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

from openai_chat import Failure, project_failure as chat_failure

SENTINEL: Final = "$API_BASE"
NO_RETRIES: Final = 0
# speech returns the audio, or a coroutine of it on its internal async path
SpeechAnswer: TypeAlias = (
    HttpxBinaryResponseContent | Coroutine[object, object, HttpxBinaryResponseContent]
)


class StreamedSpeechError(TypeError):
    def __init__(self) -> None:
        super().__init__("litellm.speech returned a coroutine for a sync call")


class Globals(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_version: str | None = None
    api_key: str | None = None
    openai_key: str | None = None
    organization: str | None = None
    project: str | None = None
    azure_key: str | None = None
    num_retries: int | None = None
    request_timeout: float | None = None


class Step(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    input: str = "hello"
    voice: str | dict[str, Json] | None = None
    api_key: str | None = None
    api_base: str | None = None
    base_url: str | None = None
    api_version: str | None = None
    organization: str | None = None
    project: str | None = None
    custom_llm_provider: str | None = None
    max_retries: int | None = None
    timeout_seconds: int | None = None
    metadata: dict[str, Json] | None = None
    response_format: str | None = None
    speed: int | None = None
    instructions: str | None = None
    headers: dict[str, Json] | None = None
    kwargs: dict[str, Json] = Field(default_factory=dict)
    lyria: bool = False
    operation_body: list[int] = Field(default_factory=list)


class SpeechCallCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    globals: Globals = Field(default_factory=Globals)
    steps: list[Step]


class BinaryValue(BaseModel):
    status: int
    body: list[int]


class StepOutcome(BaseModel):
    value: BinaryValue | None = None
    failure: Failure | None = None


class Outcome(BaseModel):
    steps: list[StepOutcome]


class SpeechCallValue(BaseModel):
    value: Outcome


class SpeechCallOutcome(BaseModel):
    speech_call: SpeechCallValue


# litellm reads provider bases and keys from the environment, so the sentinel resolves there too
def apply_environment(api_base: str) -> None:
    for name, value in list(os.environ.items()):
        if value == SENTINEL:
            os.environ[name] = api_base


def apply_globals(shared: Globals, api_base: str) -> None:
    litellm.api_base = api_base if shared.api_base == SENTINEL else shared.api_base
    litellm.api_version = shared.api_version
    litellm.api_key = shared.api_key
    litellm.openai_key = shared.openai_key
    litellm.azure_key = shared.azure_key
    litellm.num_retries = shared.num_retries


def speak(step: Step, api_base: str) -> HttpxBinaryResponseContent:
    answer: SpeechAnswer = litellm_speech(
        model=step.model,
        input=step.input,
        voice=step.voice,
        api_key=step.api_key,
        api_base=api_base if step.api_base == SENTINEL else step.api_base,
        api_version=step.api_version,
        organization=step.organization,
        project=step.project,
        max_retries=NO_RETRIES if step.max_retries is None else step.max_retries,
        metadata=step.metadata,
        timeout=None if step.timeout_seconds is None else float(step.timeout_seconds),
        response_format=step.response_format,
        speed=step.speed,
        instructions=step.instructions,
        client=None,
        headers=step.headers,
        custom_llm_provider=step.custom_llm_provider,
        base_url=api_base if step.base_url == SENTINEL else step.base_url,
        **step.kwargs,
    )
    match answer:
        case HttpxBinaryResponseContent():
            return answer
    raise StreamedSpeechError


def project_failure(error: BaseException) -> StepOutcome:
    match error:
        case openai.APIError():
            return StepOutcome(failure=chat_failure(error).failure)
        case BaseException():
            return StepOutcome(
                failure=Failure(cls=type(error).__name__, status=0, message=str(error))
            )


def run_step(step: Step, api_base: str) -> StepOutcome:
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(speak, step, api_base)
    raised = future.exception()
    match raised:
        case StreamedSpeechError():
            raise raised
        case BaseException():
            return project_failure(raised)
    answer = future.result()
    return StepOutcome(
        value=BinaryValue(status=answer.response.status_code, body=list(answer.content))
    )


def run_speech(call: SpeechCallCall, api_base: str) -> str:
    apply_environment(api_base)
    apply_globals(call.globals, api_base)
    outcomes = [run_step(step, api_base) for step in call.steps]
    outcome = SpeechCallValue(value=Outcome(steps=outcomes))
    return SpeechCallOutcome(speech_call=outcome).model_dump_json(
        by_alias=True, exclude_unset=True
    )


class SpeechCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_call: SpeechCallCall

    def serialize(self, api_base: str) -> str:
        return run_speech(self.speech_call, api_base)

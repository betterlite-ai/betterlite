import os
from concurrent.futures import ThreadPoolExecutor

import litellm
import openai
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

SENTINEL = "$API_BASE"
NO_RETRIES = 0


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


class Failure(BaseModel):
    cls: str = Field(serialization_alias="class")
    status: int
    type: str | None = None
    param: str | None = None
    code: str | None = None
    message: str


class StepOutcome(BaseModel):
    value: BinaryValue | None = None
    failure: Failure | None = None


class Outcome(BaseModel):
    steps: list[StepOutcome]


class SpeechCallValue(BaseModel):
    value: Outcome


class SpeechCallOutcome(BaseModel):
    speech_call: SpeechCallValue


def endpoint(value: str | None, api_base: str) -> str | None:
    return api_base if value == SENTINEL else value


# litellm reads provider bases and keys from the environment, so the sentinel resolves there too
def apply_environment(api_base: str) -> None:
    for name, value in list(os.environ.items()):
        if value == SENTINEL:
            os.environ[name] = api_base


def apply_globals(globals: Globals, api_base: str) -> None:
    litellm.api_base = endpoint(globals.api_base, api_base)
    litellm.api_version = globals.api_version
    litellm.api_key = globals.api_key
    litellm.openai_key = globals.openai_key
    litellm.azure_key = globals.azure_key
    litellm.num_retries = globals.num_retries


def speak(step: Step, api_base: str) -> HttpxBinaryResponseContent:
    answer = litellm.speech(
        model=step.model,
        input=step.input,
        voice=step.voice,
        api_key=step.api_key,
        api_base=endpoint(step.api_base, api_base),
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
        base_url=endpoint(step.base_url, api_base),
        **step.kwargs,
    )
    if not isinstance(answer, HttpxBinaryResponseContent):
        raise StreamedSpeechError
    return answer


def status_of(error: openai.APIError) -> int:
    value: object = getattr(error, "status_code", None)
    return value if isinstance(value, int) else 0


def text_field(error: openai.APIError, name: str) -> str | None:
    value: object = getattr(error, name, None)
    return value if isinstance(value, str) else None


def openai_failure(error: openai.APIError) -> Failure:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_of(error),
        "type": text_field(error, "type"),
        "param": text_field(error, "param"),
        "message": error.message,
    }
    code = text_field(error, "code")
    if code is not None:
        fields["code"] = code
    return Failure.model_validate(fields)


def project_failure(error: BaseException) -> StepOutcome:
    match error:
        case openai.APIError():
            return StepOutcome(failure=openai_failure(error))
        case BaseException():
            return StepOutcome(
                failure=Failure(cls=type(error).__name__, status=0, message=str(error))
            )


def run_step(step: Step, api_base: str) -> StepOutcome:
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(speak, step, api_base)
    raised = future.exception()
    if raised is not None:
        if isinstance(raised, StreamedSpeechError):
            raise raised
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

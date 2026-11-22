import asyncio
from typing import Protocol

import litellm.main as speech_main
import openai
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

from speech_call import (
    NO_RETRIES,
    SENTINEL,
    BinaryValue,
    Globals,
    Outcome,
    Step,
    StepOutcome,
    apply_environment,
    apply_globals,
    project_failure,
)


# litellm's client decorator leaves aspeech untyped, so the module is read through this protocol
class AsyncSpeechEntry(Protocol):
    async def aspeech(self, /, **kwargs: Json) -> HttpxBinaryResponseContent: ...


ENTRY: AsyncSpeechEntry = speech_main


class AsyncStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech: Step | None = None


class SpeechCallPlanCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    globals: Globals = Field(default_factory=Globals)
    steps: list[AsyncStep]


class SpeechCallPlanValue(BaseModel):
    value: Outcome


class SpeechCallPlanOutcome(BaseModel):
    speech_call_plan: SpeechCallPlanValue


async def speak(step: AsyncStep, api_base: str) -> HttpxBinaryResponseContent:
    speech = step.speech
    if speech is None:
        return await ENTRY.aspeech()
    return await ENTRY.aspeech(
        model=speech.model,
        input=speech.input,
        voice=speech.voice,
        api_key=speech.api_key,
        api_base=api_base if speech.api_base == SENTINEL else speech.api_base,
        api_version=speech.api_version,
        organization=speech.organization,
        project=speech.project,
        max_retries=NO_RETRIES if speech.max_retries is None else speech.max_retries,
        metadata=speech.metadata,
        timeout=None if speech.timeout_seconds is None else float(speech.timeout_seconds),
        response_format=speech.response_format,
        speed=speech.speed,
        instructions=speech.instructions,
        headers=speech.headers,
        custom_llm_provider=speech.custom_llm_provider,
        base_url=api_base if speech.base_url == SENTINEL else speech.base_url,
        **speech.kwargs,
    )


def run_step(step: AsyncStep, api_base: str) -> StepOutcome:
    try:
        answer = asyncio.run(speak(step, api_base))
    except (openai.APIError, KeyError) as error:
        return project_failure(error)
    return StepOutcome(
        value=BinaryValue(status=answer.response.status_code, body=list(answer.content))
    )


def run_plan(call: SpeechCallPlanCall, api_base: str) -> str:
    apply_environment(api_base)
    apply_globals(call.globals, api_base)
    outcomes = [run_step(step, api_base) for step in call.steps]
    outcome = SpeechCallPlanValue(value=Outcome(steps=outcomes))
    return SpeechCallPlanOutcome(speech_call_plan=outcome).model_dump_json(
        by_alias=True, exclude_unset=True
    )


class SpeechCallPlanRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    speech_call_plan: SpeechCallPlanCall

    def serialize(self, api_base: str) -> str:
        return run_plan(self.speech_call_plan, api_base)

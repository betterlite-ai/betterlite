import asyncio
from collections.abc import Callable, Coroutine
from typing import Literal

import litellm
import litellm.assistants.main as runs
import openai
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent
from openai.types.beta.threads import Run
from pydantic import BaseModel, ConfigDict

from openai_chat import Failure, optional_text, status_code

RUN: Callable[..., Run] = runs.run_thread
ARUN: Callable[..., Coroutine[object, object, Run]] = runs.arun_thread
RUN_STREAM: Callable[..., object] = runs.run_thread_stream
ARUN_STREAM: Callable[..., object] = runs.arun_thread_stream


class Recorder(openai.AssistantEventHandler):
    def __init__(self) -> None:
        super().__init__()
        self.events: list[str] = []
        self.ended = False

    def on_event(self, event: AssistantStreamEvent) -> None:
        self.events.append(event.event)

    def on_end(self) -> None:
        self.ended = True


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    mode: Literal["run", "run_stream", "arun", "arun_stream"]
    provider: str
    thread_id: str
    assistant_id: str
    additional_instructions: str | None = None
    instructions: str | None = None
    metadata: object = None
    model: str | None = None
    stream: object = None
    tools: object = None
    event_handler: bool = False
    arun_thread: object = None
    params: dict[str, object] = {}


class RunView(BaseModel):
    id: str
    status: str
    tool_calls: int | None
    last_error: dict[str, str | None] | None


class HandlerView(BaseModel):
    events: list[str]
    ended: bool


class StreamView(BaseModel):
    events: list[str]
    handler: HandlerView | None


class ResponseView(BaseModel):
    content: str
    method: str


class Outcome(BaseModel):
    run: RunView | None
    stream: StreamView | None
    failure: Failure | None
    response: ResponseView | None


class AssistantRunValue(BaseModel):
    value: Outcome


class Result(BaseModel):
    assistant_run: AssistantRunValue


class AssistantRunRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_run: Call

    def serialize(self, api_base: str) -> str:
        value = run(self.assistant_run, api_base)
        return Result(assistant_run=AssistantRunValue(value=value)).model_dump_json(
            by_alias=True, exclude_unset=True
        )


def body(call: Call, api_base: str, handler: Recorder | None) -> dict[str, object]:
    values: dict[str, object] = {
        "custom_llm_provider": call.provider,
        "thread_id": call.thread_id,
        "assistant_id": call.assistant_id,
        "additional_instructions": call.additional_instructions,
        "instructions": call.instructions,
        "metadata": call.metadata,
        "model": call.model,
        "tools": call.tools,
        "event_handler": handler,
        "arun_thread": call.arun_thread,
        **call.params,
        "api_base": api_base,
    }
    if "stream" in call.model_fields_set:
        values["stream"] = call.stream
    return values


def run(call: Call, api_base: str) -> Outcome:
    handler = Recorder() if call.event_handler else None
    kwargs = body(call, api_base, handler)
    result: object
    try:
        if call.mode == "run_stream":
            result = RUN_STREAM(**kwargs)
        elif call.mode == "arun_stream":
            result = ARUN_STREAM(**kwargs)
        elif call.mode == "arun":
            result = asyncio.run(ARUN(**kwargs))
        else:
            result = RUN(**kwargs)
    except (openai.APIError, TypeError, ValueError) as error:
        return failure_outcome(error)
    if isinstance(result, AssistantStreamManager):
        manager: AssistantStreamManager[openai.AssistantEventHandler] = result
        with manager as stream:
            return stream_outcome([event.event for event in stream], handler)
    if isinstance(result, AsyncAssistantStreamManager):
        async_manager: AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler] = result
        return stream_outcome(asyncio.run(drain(async_manager)), handler)
    return run_outcome(result)


async def drain(
    manager: AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler],
) -> list[str]:
    async with manager as stream:
        return [event.event async for event in stream]


def stream_outcome(events: list[str], handler: Recorder | None) -> Outcome:
    view = None if handler is None else HandlerView(events=handler.events, ended=handler.ended)
    stream = StreamView(events=events, handler=view)
    return Outcome(run=None, stream=stream, failure=None, response=None)


def run_outcome(response: object) -> Outcome:
    if not isinstance(response, Run):
        raise RuntimeError(f"the wrapper returned {type(response).__name__} instead of a run")
    tool_calls = None
    if response.required_action is not None:
        tool_calls = len(response.required_action.submit_tool_outputs.tool_calls)
    last_error: dict[str, str | None] | None = None
    if response.last_error is not None:
        last_error = {"code": response.last_error.code, "message": response.last_error.message}
    view = RunView(
        id=response.id,
        status=response.status,
        tool_calls=tool_calls,
        last_error=last_error,
    )
    return Outcome(run=view, stream=None, failure=None, response=None)


def failure_outcome(error: openai.APIError | TypeError | ValueError) -> Outcome:
    if not isinstance(error, openai.APIError):
        failure = Failure(cls=type(error).__name__, status=0, message=str(error))
        return Outcome(run=None, stream=None, failure=failure, response=None)
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    response = None
    if isinstance(error, litellm.BadRequestError):
        content = error.response.content.decode()
        response = ResponseView(content=content, method=error.response.request.method)
    return Outcome(
        run=None,
        stream=None,
        failure=Failure.model_validate(fields),
        response=response,
    )

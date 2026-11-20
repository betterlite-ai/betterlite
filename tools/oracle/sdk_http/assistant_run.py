import asyncio
from typing import Literal

import litellm.assistants.main as runs
import openai
from litellm.exceptions import BadRequestError
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent
from openai.types.beta.threads import Run
from pydantic import BaseModel, ConfigDict, JsonValue as Json
from typing_extensions import override

from openai_chat import Failure, optional_text, status_code


class Recorder(openai.AssistantEventHandler):
    def __init__(self) -> None:
        super().__init__()
        self.events: list[str] = []
        self.ended = False

    @override
    def on_event(self, event: AssistantStreamEvent) -> None:
        self.events.append(event.event)

    @override
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
    metadata: Json = None
    model: str | None = None
    stream: Json = None
    tools: Json = None
    event_handler: bool = False
    arun_thread: Json = None
    params: dict[str, Json] = {}


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


def request_options(call: Call, api_base: str) -> dict[str, Json]:
    values: dict[str, Json] = {
        "custom_llm_provider": call.provider,
        "thread_id": call.thread_id,
        "assistant_id": call.assistant_id,
        "additional_instructions": call.additional_instructions,
        "instructions": call.instructions,
        "metadata": call.metadata,
        "model": call.model,
        "tools": call.tools,
        "arun_thread": call.arun_thread,
        **call.params,
        "api_base": api_base,
    }
    if "stream" in call.model_fields_set:
        values["stream"] = call.stream
    return values


def run(call: Call, api_base: str) -> Outcome:
    handler = Recorder() if call.event_handler else None
    options = request_options(call, api_base)
    try:
        match call.mode:
            case "run":
                return run_call(call, options, handler)
            case "arun":
                return arun_call(call, options, handler)
            case "run_stream":
                return run_stream_call(call, options, handler)
            case "arun_stream":
                manager = runs.arun_thread_stream(**options, event_handler=handler)
                return async_manager(manager, handler)
    except (TypeError, ValueError) as error:
        failure = Failure(cls=error.__class__.__name__, status=0, message=str(error))
        return Outcome(run=None, stream=None, failure=failure, response=None)
    except openai.APIError as error:
        return api_failure(call, error)


def run_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.stream is not True:
        response: Run = runs.run_thread(**options, event_handler=handler)
        return run_outcome(response)
    stream_options = {name: value for name, value in options.items() if name != "stream"}
    if call.arun_thread is True:
        manager = runs.arun_thread_stream(**stream_options, event_handler=handler)
        return async_manager(manager, handler)
    sync = runs.run_thread_stream(**stream_options, event_handler=handler)
    return sync_manager(sync, handler)


def arun_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.stream is not True:
        response: Run = asyncio.run(runs.arun_thread(**options, event_handler=handler))
        return run_outcome(response)
    stream_options = {name: value for name, value in options.items() if name != "stream"}
    manager = runs.arun_thread_stream(**stream_options, event_handler=handler)
    return async_manager(manager, handler)


def run_stream_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.arun_thread is True:
        manager = runs.arun_thread_stream(**options, event_handler=handler)
        return async_manager(manager, handler)
    sync = runs.run_thread_stream(**options, event_handler=handler)
    return sync_manager(sync, handler)


def sync_manager(
    manager: AssistantStreamManager[openai.AssistantEventHandler],
    handler: Recorder | None,
) -> Outcome:
    with manager as stream:
        return stream_outcome([event.event for event in stream], handler)


def async_manager(
    manager: AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler],
    handler: Recorder | None,
) -> Outcome:
    events = asyncio.run(drain(manager))
    return stream_outcome(events, handler)


async def drain(
    manager: AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler],
) -> list[str]:
    async with manager as stream:
        return [event.event async for event in stream]


def stream_outcome(events: list[str], handler: Recorder | None) -> Outcome:
    view = None if handler is None else HandlerView(events=handler.events, ended=handler.ended)
    stream = StreamView(events=events, handler=view)
    return Outcome(run=None, stream=stream, failure=None, response=None)


def run_outcome(response: Run) -> Outcome:
    required = response.required_action
    tool_calls = None if required is None else len(required.submit_tool_outputs.tool_calls)
    reported = response.last_error
    last_error = None if reported is None else {"code": reported.code, "message": reported.message}
    view = RunView(
        id=response.id,
        status=response.status,
        tool_calls=tool_calls,
        last_error=last_error,
    )
    return Outcome(run=view, stream=None, failure=None, response=None)


def api_failure(call: Call, error: openai.APIError) -> Outcome:
    fields: dict[str, str | int | None] = {
        "cls": error.__class__.__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    failure = Failure.model_validate(fields)
    if call.provider not in ("openai", "azure"):
        match error:
            case BadRequestError():
                response = ResponseView(
                    content=error.response.content.decode(),
                    method=error.response.request.method,
                )
                return Outcome(run=None, stream=None, failure=failure, response=response)
    return Outcome(run=None, stream=None, failure=failure, response=None)

import asyncio
from collections.abc import Coroutine
from typing import Literal, Protocol, cast

import litellm.assistants.main as runs
import openai
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent
from openai.types.beta.threads import Run
from pydantic import BaseModel, ConfigDict
from pydantic import JsonValue as Json
from typing_extensions import override

from openai_chat import Failure, project_failure


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


# the wrappers have no return annotation, so their inferred Any goes through this facade
class Runs(Protocol):
    def run_thread(self, *, event_handler: Recorder | None = None, **kwargs: Json) -> Run: ...

    def arun_thread(
        self, *, event_handler: Recorder | None = None, **kwargs: Json
    ) -> Coroutine[None, None, Run]: ...

    def run_thread_stream(
        self, *, event_handler: Recorder | None = None, **kwargs: Json
    ) -> AssistantStreamManager[openai.AssistantEventHandler]: ...

    def arun_thread_stream(
        self, *, event_handler: Recorder | None = None, **kwargs: Json
    ) -> AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler]: ...


RUNS = cast(Runs, runs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1165-L1365


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
    last_error: dict[str, str] | None


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


def run(call: Call, api_base: str) -> Outcome:
    handler = Recorder() if call.event_handler else None
    options: dict[str, Json] = {
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
        options["stream"] = call.stream
    try:
        return mode_outcome(call, options, handler)
    except (TypeError, ValueError) as error:
        failure = Failure(cls=type(error).__name__, status=0, message=str(error))
        return Outcome(run=None, stream=None, failure=failure, response=None)
    except openai.BadRequestError as error:
        response = (
            ResponseView(
                content=error.response.content.decode(),
                method=error.response.request.method,
            )
            if call.provider not in ("openai", "azure")
            else None
        )
        return api_failure(error, response)
    except openai.APIError as error:
        return api_failure(error)


def mode_outcome(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    match call.mode:
        case "run":
            return run_call(call, options, handler)
        case "arun":
            return arun_call(call, options, handler)
        case "run_stream":
            return run_stream_call(call, options, handler)
        case "arun_stream":
            manager = RUNS.arun_thread_stream(**options, event_handler=handler)
            return async_manager(manager, handler)


def api_failure(error: openai.APIError, response: ResponseView | None = None) -> Outcome:
    return Outcome(
        run=None,
        stream=None,
        failure=project_failure(error).failure,
        response=response,
    )


def run_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.stream is not True:
        response: Run = RUNS.run_thread(**options, event_handler=handler)
        return run_outcome(response)
    stream_options = {name: value for name, value in options.items() if name != "stream"}
    if call.arun_thread is True:
        manager = RUNS.arun_thread_stream(**stream_options, event_handler=handler)
        return async_manager(manager, handler)
    sync = RUNS.run_thread_stream(**stream_options, event_handler=handler)
    return sync_manager(sync, handler)


def arun_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.stream is not True:
        held = RUNS.arun_thread(**options, event_handler=handler)
        response: Run = asyncio.run(held)
        return run_outcome(response)
    stream_options = {name: value for name, value in options.items() if name != "stream"}
    manager = RUNS.arun_thread_stream(**stream_options, event_handler=handler)
    return async_manager(manager, handler)


def run_stream_call(call: Call, options: dict[str, Json], handler: Recorder | None) -> Outcome:
    if call.arun_thread is True:
        manager = RUNS.arun_thread_stream(**options, event_handler=handler)
        return async_manager(manager, handler)
    sync = RUNS.run_thread_stream(**options, event_handler=handler)
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

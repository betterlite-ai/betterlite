import asyncio

import openai
from litellm.llms.openai.openai import OpenAIAssistantsAPI
from pydantic import JsonValue

from .assistants import assistants_answer
from .decisions import decisions_answer
from .realtime import realtime_answer
from .support import (
    AsyncFacade,
    Helpers,
    Input,
    SyncFacade,
    Wire,
    async_client,
    credentials,
    sync_client,
)
from .threads import threads_answer

STREAMS: Helpers = OpenAIAssistantsAPI()
SYNC_RUNS: SyncFacade = OpenAIAssistantsAPI()
ASYNC_RUNS: AsyncFacade = OpenAIAssistantsAPI()


# The stream manager sends nothing until it is entered, so a refused open surfaces while reading.
async def stream_answer(input_value: Input, wire: Wire) -> dict[str, JsonValue]:
    manager = STREAMS.async_run_thread_stream(
        **input_value["request"], client=async_client(input_value, wire), event_handler=None
    )
    sent_at_dispatch = len(wire.seen)
    names: list[JsonValue] = []
    error_status: int | None = None
    try:
        async with manager as stream:
            names = [event.event async for event in stream]
    except openai.APIStatusError as error:
        error_status = error.status_code
    return {
        "kind": "stream",
        "sent_at_dispatch": sent_at_dispatch,
        "events": names,
        "error_status": error_status,
    }


def runs_answer(input_value: Input, wire: Wire) -> dict[str, JsonValue]:
    if input_value["operation"] == "stream":
        return asyncio.run(stream_answer(input_value, wire))
    body = input_value["request"]
    if input_value["arun_thread"]:
        polled = ASYNC_RUNS.run_thread(
            **body,
            **credentials(input_value),
            stream=False,
            client=async_client(input_value, wire),
            arun_thread=True,
            event_handler=None,
        )
        run_object = asyncio.run(polled)
    else:
        run_object = SYNC_RUNS.run_thread(
            **body,
            **credentials(input_value),
            stream=False,
            client=sync_client(input_value, wire),
            arun_thread=False,
            event_handler=None,
        )
    return {"kind": "polled", "id": run_object.id, "status": run_object.status}


def run(input_value: Input) -> dict[str, JsonValue]:
    kind = input_value["kind"]
    wire = Wire(input_value["replies"])
    result: JsonValue
    if kind == "decisions":
        result = decisions_answer(input_value)
    elif kind == "realtime":
        result = realtime_answer(input_value)
    elif kind == "assistants":
        result = assistants_answer(input_value, wire)
    elif kind == "threads":
        result = asyncio.run(threads_answer(input_value, wire))
    else:
        result = runs_answer(input_value, wire)
    return {"requests": wire.seen, "result": result}

import asyncio
import functools
import inspect
import json
import os
from collections.abc import Coroutine
from dataclasses import dataclass, field
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm.llms.openai.openai as litellm_openai
import openai
import pydantic
from litellm.llms.openai.openai import OpenAIAssistantsAPI
from openai import AssistantEventHandler, AsyncAssistantEventHandler
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Reply: TypeAlias = tuple[int, dict[str, str], str]
Options: TypeAlias = tuple[str | None, str | None, float | None, int | None, str | None]

NO_REPLY = '{"error":{"message":"the fixture queues no reply"}}'


@dataclass
class Wire:
    replies: list[Reply]
    seen: list[Json] = field(default_factory=lambda: cast(list[Json], []))

    def handle(self, request: httpx.Request) -> httpx.Response:
        content = request.content
        self.seen.append(
            {
                "method": request.method,
                "url": str(request.url),
                "authorization": request.headers.get("authorization"),
                "openai-organization": request.headers.get("openai-organization"),
                "openai-beta": request.headers.get("openai-beta"),
                "content-type": request.headers.get("content-type"),
                "body": cast(Json, json.loads(content)) if content else None,
            }
        )
        status, headers, body = self.replies.pop(0) if self.replies else (500, {}, NO_REPLY)
        return httpx.Response(status, content=body, headers=headers)


Calls: TypeAlias = list[str]


class Recorder(AssistantEventHandler):
    def __init__(self, calls: Calls) -> None:
        super().__init__()
        self.calls = calls

    def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.append(event_name(event))

    def on_exception(self, exception: Exception) -> None:
        self.calls.append("exception:" + error_label(exception))

    def on_timeout(self) -> None:
        self.calls.append("timeout")

    def on_end(self) -> None:
        self.calls.append("end")


class AsyncRecorder(AsyncAssistantEventHandler):
    def __init__(self, calls: Calls) -> None:
        super().__init__()
        self.calls = calls

    async def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.append(event_name(event))

    async def on_exception(self, exception: Exception) -> None:
        self.calls.append("exception:" + error_label(exception))

    async def on_timeout(self) -> None:
        self.calls.append("timeout")

    async def on_end(self) -> None:
        self.calls.append("end")


def object_of(raw: Json) -> Object:
    if not isinstance(raw, dict):
        raise TypeError("expected an object")
    return cast(Object, raw)


def error_label(error: Exception) -> str:
    if isinstance(error, openai.APIStatusError):
        return "status"
    if isinstance(error, openai.APIError):
        return "stream_error"
    return "other"


def failure(error: Exception) -> Json:
    if isinstance(error, openai.APIStatusError):
        return {"kind": "status", "status": error.status_code}
    if isinstance(error, json.JSONDecodeError):
        return {"kind": "malformed"}
    if isinstance(error, openai.APIError):
        body = cast(object, error.body)
        if isinstance(body, dict):
            fields = cast("dict[str, Json]", body)
            return {"kind": "stream_error", "type": fields.get("type"), "code": fields.get("code")}
    if isinstance(error, pydantic.ValidationError):
        return {"kind": "shape"}
    if isinstance(error, openai.OpenAIError):
        return {"kind": "configuration"}
    if isinstance(error, ValueError):
        return {"kind": "invalid_thread_id"}
    raise error


def event_name(event: object) -> str:
    if not isinstance(event, openai.BaseModel):
        return "payload"
    name = cast("str | None", cast(AssistantStreamEvent, event).event)
    return name if name is not None else "payload"


def event_dump(event: object) -> Json:
    if not isinstance(event, openai.BaseModel):
        return cast(Json, event)
    stream_event = cast(AssistantStreamEvent, event)
    name = cast("str | None", stream_event.event)
    typed = name is not None and name.startswith(("thread.run.", "thread.message."))
    if typed:
        return cast(Json, stream_event.model_dump())
    return cast(Json, stream_event.model_dump(exclude_none=True))


def sdk_clients(wire: Wire) -> tuple[object, object]:
    sync = functools.partial(
        openai.OpenAI, http_client=httpx.Client(transport=httpx.MockTransport(wire.handle))
    )
    asynchronous = functools.partial(
        openai.AsyncOpenAI,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire.handle)),
    )
    return sync, asynchronous


def consume_sync(manager: object, wire: Wire, calls: Calls) -> Json:
    sync = cast(AssistantStreamManager[AssistantEventHandler], manager)
    body: Object = {"selected": "sync_stream", "requests": cast(Json, wire.seen)}
    body["calls"] = cast(Json, calls)
    try:
        with sync as entered:
            events = [event_dump(event) for event in entered]
    except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
        body["error"] = failure(error)
        return body
    body["events"] = cast(Json, events)
    return body


def consume_async(manager: object, wire: Wire, calls: Calls) -> Json:
    asynchronous = cast(AsyncAssistantStreamManager[AsyncAssistantEventHandler], manager)
    body: Object = {"selected": "async_stream", "requests": cast(Json, wire.seen)}
    body["calls"] = cast(Json, calls)

    async def drain() -> list[Json]:
        async with asynchronous as entered:
            return [event_dump(event) async for event in entered]

    try:
        events = asyncio.run(drain())
    except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
        body["error"] = failure(error)
        return body
    body["events"] = cast(Json, events)
    return body


def buffered(selected: str, wire: Wire, result: object) -> Json:
    if inspect.iscoroutine(result):
        run = asyncio.run(cast(Coroutine[object, object, openai.BaseModel], result))
    else:
        run = result
    return {
        "selected": selected,
        "requests": cast(Json, wire.seen),
        "result": cast(Json, cast(openai.BaseModel, run).model_dump()),
    }


def call_run_thread(
    api: OpenAIAssistantsAPI,
    options: Options,
    request: Object,
    round_value: Object,
    handler: object,
) -> object:
    return cast(object, api.run_thread(  # pyright: ignore[reportCallIssue, reportUnknownMemberType]
        thread_id=cast(str, request["thread_id"]),
        assistant_id=cast(str, request["assistant_id"]),
        additional_instructions=cast("str | None", request.get("additional_instructions")),
        instructions=cast("str | None", request.get("instructions")),
        metadata=cast("dict[str, str] | None", request.get("metadata")),
        model=cast("str | None", request.get("model")),
        stream=cast("bool | None", round_value.get("stream")),
        tools=cast("object", request.get("tools")),  # pyright: ignore[reportArgumentType]
        api_key=options[0],
        api_base=options[1],
        timeout=options[2],  # pyright: ignore[reportArgumentType]
        max_retries=options[3],
        organization=options[4],
        client=None,
        arun_thread=cast("bool | None", round_value.get("arun_thread")),  # pyright: ignore[reportArgumentType]
        event_handler=handler,  # pyright: ignore[reportArgumentType]
    ))


def run_round(fields: Object, round_value: Object) -> Json:
    settings = object_of(fields.get("options"))
    timeout = settings.get("timeout")
    options: Options = (
        cast("str | None", settings.get("api_key")),
        cast("str | None", settings.get("api_base")),
        None if timeout is None else float(cast(float, timeout)),
        cast("int | None", settings.get("max_retries")),
        cast("str | None", settings.get("organization")),
    )
    replies: list[Reply] = [
        (
            cast(int, object_of(entry)["status"]),
            cast("dict[str, str]", object_of(entry).get("headers") or {}),
            cast(str, object_of(entry)["body"]),
        )
        for entry in cast("list[Json]", round_value.get("replies") or [])
    ]
    wire = Wire(replies)
    sync, asynchronous = sdk_clients(wire)
    request = object_of(round_value["request"])
    arun_thread = round_value.get("arun_thread")
    supplied = bool(round_value.get("handler"))
    calls: Calls = []
    handler: object = None
    if supplied:
        handler = AsyncRecorder(calls) if arun_thread is True else Recorder(calls)
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            result = call_run_thread(OpenAIAssistantsAPI(), options, request, round_value, handler)
            if inspect.iscoroutine(result):
                return buffered("async_run", wire, result)
            manager: object = result
            if isinstance(result, (AssistantStreamManager, AsyncAssistantStreamManager)):
                if isinstance(result, AssistantStreamManager):
                    return consume_sync(manager, wire, calls)
                return consume_async(manager, wire, calls)
            return buffered("sync_run", wire, result)
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": cast(Json, wire.seen), "error": failure(error)}


def run(input_value: dict[str, Json]) -> Json:
    ((_variant, fields),) = input_value.items()
    rounds = cast("list[Json]", object_of(fields)["rounds"])
    outcomes = [run_round(object_of(fields), object_of(round_value)) for round_value in rounds]
    return {"rounds": cast(Json, outcomes)}

import asyncio
import functools
import inspect
import json
import os
from collections.abc import Coroutine
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

NO_REPLY = '{"error":{"message":"the fixture queues no reply"}}'


def obj(raw: Json) -> Object:
    return cast(Object, raw)


def wire(replies: list[Reply]) -> tuple[httpx.MockTransport, list[Json]]:
    seen: list[Json] = []

    def handle(request: httpx.Request) -> httpx.Response:
        seen.append(
            {
                "method": request.method,
                "url": str(request.url),
                "authorization": request.headers.get("authorization"),
                "openai-organization": request.headers.get("openai-organization"),
                "openai-beta": request.headers.get("openai-beta"),
                "content-type": request.headers.get("content-type"),
                "body": cast(Json, json.loads(request.content)) if request.content else None,
            }
        )
        status, headers, body = replies.pop(0) if replies else (500, {}, NO_REPLY)
        return httpx.Response(status, content=body, headers=headers)

    return httpx.MockTransport(handle), seen


class Recorder(AssistantEventHandler):
    def __init__(self, calls: list[str]) -> None:
        super().__init__()
        self.calls = calls

    def on_event(self, event: AssistantStreamEvent) -> None:
        name = cast("str | None", event.event)
        self.calls.append(name if name is not None else "payload")

    def on_exception(self, exception: Exception) -> None:
        label = "stream_error" if isinstance(exception, openai.APIError) else "other"
        self.calls.append("exception:" + label)

    def on_timeout(self) -> None:
        self.calls.append("timeout")

    def on_end(self) -> None:
        self.calls.append("end")


class AsyncRecorder(AsyncAssistantEventHandler):
    def __init__(self, calls: list[str]) -> None:
        super().__init__()
        self.calls = calls

    async def on_event(self, event: AssistantStreamEvent) -> None:
        name = cast("str | None", event.event)
        self.calls.append(name if name is not None else "payload")

    async def on_exception(self, exception: Exception) -> None:
        label = "stream_error" if isinstance(exception, openai.APIError) else "other"
        self.calls.append("exception:" + label)

    async def on_timeout(self) -> None:
        self.calls.append("timeout")

    async def on_end(self) -> None:
        self.calls.append("end")


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


def event_dump(event: object) -> Json:
    if not isinstance(event, openai.BaseModel):
        return cast(Json, event)
    stream_event = cast(AssistantStreamEvent, event)
    name = cast("str | None", stream_event.event)
    typed = name is not None and name.startswith(("thread.run.", "thread.message."))
    if typed:
        return cast(Json, stream_event.model_dump())
    return cast(Json, stream_event.model_dump(exclude_none=True))


async def drain(manager: AsyncAssistantStreamManager[AsyncAssistantEventHandler]) -> list[Json]:
    async with manager as entered:
        return [event_dump(event) async for event in entered]


def consume(
    manager: object, seen: list[Json], calls: list[str], *, asynchronous: bool
) -> Json:
    body: Object = {
        "selected": "async_stream" if asynchronous else "sync_stream",
        "requests": cast(Json, seen),
        "calls": cast(Json, calls),
    }
    try:
        if asynchronous:
            queued = cast(AsyncAssistantStreamManager[AsyncAssistantEventHandler], manager)
            events = asyncio.run(drain(queued))
        else:
            with cast(AssistantStreamManager[AssistantEventHandler], manager) as entered:
                events = [event_dump(event) for event in entered]
    except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
        body["error"] = failure(error)
        return body
    body["events"] = cast(Json, events)
    return body


def buffered(selected: str, seen: list[Json], result: object) -> Json:
    if inspect.iscoroutine(result):
        result = asyncio.run(cast(Coroutine[object, object, openai.BaseModel], result))
    dumped = cast(openai.BaseModel, result).model_dump()
    return {"selected": selected, "requests": cast(Json, seen), "result": cast(Json, dumped)}


def call_run_thread(
    api: OpenAIAssistantsAPI,
    settings: Object,
    request: Object,
    round_value: Object,
    handler: object,
) -> object:
    timeout = settings.get("timeout")
    return cast(object, api.run_thread(  # pyright: ignore[reportCallIssue, reportUnknownMemberType]
        thread_id=cast(str, request["thread_id"]),
        assistant_id=cast(str, request["assistant_id"]),
        additional_instructions=cast("str | None", request.get("additional_instructions")),
        instructions=cast("str | None", request.get("instructions")),
        metadata=cast("dict[str, str] | None", request.get("metadata")),
        model=cast("str | None", request.get("model")),
        stream=cast("bool | None", round_value.get("stream")),
        tools=cast("object", request.get("tools")),  # pyright: ignore[reportArgumentType]
        api_key=cast("str | None", settings.get("api_key")),
        api_base=cast("str | None", settings.get("api_base")),
        timeout=None if timeout is None else float(cast(float, timeout)),  # pyright: ignore[reportArgumentType]
        max_retries=cast("int | None", settings.get("max_retries")),
        organization=cast("str | None", settings.get("organization")),
        client=None,
        arun_thread=cast("bool | None", round_value.get("arun_thread")),  # pyright: ignore[reportArgumentType]
        event_handler=handler,  # pyright: ignore[reportArgumentType]
    ))


def run_round(fields: Object, round_value: Object) -> Json:
    settings = obj(fields.get("options"))
    replies: list[Reply] = [
        (
            cast(int, obj(entry)["status"]),
            cast("dict[str, str]", obj(entry).get("headers") or {}),
            cast(str, obj(entry)["body"]),
        )
        for entry in cast("list[Json]", round_value.get("replies") or [])
    ]
    transport, seen = wire(replies)
    sync = functools.partial(openai.OpenAI, http_client=httpx.Client(transport=transport))
    asynchronous = functools.partial(
        openai.AsyncOpenAI, http_client=httpx.AsyncClient(transport=transport)
    )
    request = obj(round_value["request"])
    arun_thread = round_value.get("arun_thread")
    calls: list[str] = []
    handler: object = None
    if round_value.get("handler"):
        handler = AsyncRecorder(calls) if arun_thread is True else Recorder(calls)
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            result = call_run_thread(
                OpenAIAssistantsAPI(), settings, request, round_value, handler
            )
            if inspect.iscoroutine(result):
                return buffered("async_run", seen, result)
            if isinstance(result, AssistantStreamManager):
                answered = cast(AssistantStreamManager[AssistantEventHandler], result)
                return consume(answered, seen, calls, asynchronous=False)
            if isinstance(result, AsyncAssistantStreamManager):
                queued = cast(AsyncAssistantStreamManager[AsyncAssistantEventHandler], result)
                return consume(queued, seen, calls, asynchronous=True)
            return buffered("sync_run", seen, result)
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": cast(Json, seen), "error": failure(error)}


def run(input_value: dict[str, Json]) -> Json:
    ((_variant, fields),) = input_value.items()
    rounds = cast("list[Json]", obj(fields)["rounds"])
    outcomes = [run_round(obj(fields), obj(round_value)) for round_value in rounds]
    return {"rounds": cast(Json, outcomes)}

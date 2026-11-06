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

NO_REPLY = '{"error":{"message":"the fixture queues no reply"}}'


@dataclass
class Reply:
    status: int
    headers: dict[str, str]
    body: str


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
        reply = self.replies.pop(0) if self.replies else Reply(500, {}, NO_REPLY)
        return httpx.Response(reply.status, content=reply.body, headers=reply.headers)


@dataclass
class Calls:
    entries: list[str] = field(default_factory=lambda: cast(list[str], []))


class Recorder(AssistantEventHandler):
    def __init__(self, calls: Calls) -> None:
        super().__init__()
        self.calls = calls

    def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.entries.append(event.event)

    def on_end(self) -> None:
        self.calls.entries.append("end")


class AsyncRecorder(AsyncAssistantEventHandler):
    def __init__(self, calls: Calls) -> None:
        super().__init__()
        self.calls = calls

    async def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.entries.append(event.event)

    async def on_end(self) -> None:
        self.calls.entries.append("end")


@dataclass
class Options:
    api_key: str | None
    api_base: str | None
    timeout: float | None
    max_retries: int | None
    organization: str | None


def object_of(raw: Json) -> Object:
    if not isinstance(raw, dict):
        raise TypeError("expected an object")
    return cast(Object, raw)


def options_of(raw: Json) -> Options:
    fields = object_of(raw)
    timeout = fields.get("timeout")
    return Options(
        api_key=cast("str | None", fields.get("api_key")),
        api_base=cast("str | None", fields.get("api_base")),
        timeout=None if timeout is None else float(cast(float, timeout)),
        max_retries=cast("int | None", fields.get("max_retries")),
        organization=cast("str | None", fields.get("organization")),
    )


def replies_of(raw: Json) -> list[Reply]:
    entries = cast("list[Json]", raw or [])
    replies: list[Reply] = []
    for entry in entries:
        fields = object_of(entry)
        replies.append(
            Reply(
                status=cast(int, fields["status"]),
                headers=cast("dict[str, str]", fields.get("headers") or {}),
                body=cast(str, fields["body"]),
            )
        )
    return replies


def failure(error: Exception) -> Json:
    if isinstance(error, openai.APIStatusError):
        return {"kind": "status", "status": error.status_code}
    if isinstance(error, pydantic.ValidationError):
        return {"kind": "shape"}
    if isinstance(error, openai.OpenAIError):
        return {"kind": "configuration"}
    if isinstance(error, ValueError):
        return {"kind": "invalid_thread_id"}
    raise error


def event_dump(event: object) -> Json:
    return cast(Json, cast(openai.BaseModel, event).model_dump(exclude_none=True))


def sdk_clients(wire: Wire) -> tuple[object, object]:
    sync = functools.partial(
        openai.OpenAI, http_client=httpx.Client(transport=httpx.MockTransport(wire.handle))
    )
    asynchronous = functools.partial(
        openai.AsyncOpenAI,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire.handle)),
    )
    return sync, asynchronous


def manager_stores(manager: object, name: str, handler: object) -> bool:
    return cast(object, getattr(manager, name)) is handler


def handler_name(manager: object) -> str:
    if isinstance(manager, AssistantStreamManager):
        return "_AssistantStreamManager__event_handler"
    return "_AsyncAssistantStreamManager__event_handler"


def handler_kind(manager: object, *, supplied: bool, handler: object) -> str:
    if supplied and manager_stores(manager, handler_name(manager), handler):
        return "supplied"
    return "default"


def stream_body(selected: str, kind: str, before: int, wire: Wire, calls: Calls) -> Object:
    return {
        "selected": selected,
        "handler": kind,
        "requests_before": before,
        "requests": cast(Json, wire.seen),
        "calls": cast(Json, calls.entries),
    }


def consume_sync(manager: object, wire: Wire, kind: str, calls: Calls) -> Json:
    sync = cast(AssistantStreamManager[AssistantEventHandler], manager)
    body = stream_body("sync_stream", kind, len(wire.seen), wire, calls)
    try:
        with sync as entered:
            events = [event_dump(event) for event in entered]
    except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
        body["error"] = failure(error)
        return body
    body["events"] = cast(Json, events)
    return body


def consume_async(manager: object, wire: Wire, kind: str, calls: Calls) -> Json:
    asynchronous = cast(AsyncAssistantStreamManager[AsyncAssistantEventHandler], manager)
    body = stream_body("async_stream", kind, len(wire.seen), wire, calls)

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
        api_key=options.api_key,
        api_base=options.api_base,
        timeout=options.timeout,  # pyright: ignore[reportArgumentType]
        max_retries=options.max_retries,
        organization=options.organization,
        client=None,
        arun_thread=cast("bool | None", round_value.get("arun_thread")),  # pyright: ignore[reportArgumentType]
        event_handler=handler,  # pyright: ignore[reportArgumentType]
    ))


def buffered(selected: str, wire: Wire, result: object) -> Json:
    if inspect.iscoroutine(result):
        run = asyncio.run(cast(Coroutine[object, object, openai.BaseModel], result))
    else:
        run = result
    return {
        "selected": selected,
        "requests": cast(Json, wire.seen),
        "result": cast(Json, cast(openai.BaseModel, run).model_dump(exclude_none=True)),
    }


def run_round(fields: Object, round_value: Object) -> Json:
    options = options_of(fields.get("options"))
    wire = Wire(replies_of(round_value.get("replies")))
    sync, asynchronous = sdk_clients(wire)
    request = object_of(round_value["request"])
    arun_thread = round_value.get("arun_thread")
    supplied = bool(round_value.get("handler"))
    calls = Calls()
    handler: object = None
    if supplied:
        handler = AsyncRecorder(calls) if arun_thread is True else Recorder(calls)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            result = call_run_thread(api, options, request, round_value, handler)
            if inspect.iscoroutine(result):
                return buffered("async_run", wire, result)
            manager: object = result
            if isinstance(result, (AssistantStreamManager, AsyncAssistantStreamManager)):
                kind = handler_kind(manager, supplied=supplied, handler=handler)
                if isinstance(result, AssistantStreamManager):
                    return consume_sync(manager, wire, kind, calls)
                return consume_async(manager, wire, kind, calls)
            return buffered("sync_run", wire, result)
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": cast(Json, wire.seen), "error": failure(error)}


def run(input_value: dict[str, Json]) -> Json:
    ((_variant, fields),) = input_value.items()
    rounds = cast("list[Json]", object_of(fields)["rounds"])
    outcomes = [run_round(object_of(fields), object_of(round_value)) for round_value in rounds]
    return {"rounds": cast(Json, outcomes)}

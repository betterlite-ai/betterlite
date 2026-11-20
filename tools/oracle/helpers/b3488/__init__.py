import asyncio
import base64

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from pydantic import JsonValue

from .answer import stream_answer, unary_answer, views_of
from .call import ASYNC_STREAM, ASYNC_UNARY, STREAM, UNARY, AsyncEvents, SyncEvents, invoke
from .case import Case
from .transport import Recorder, Run, logging_object


def sync_events(stream: SyncEvents) -> list[JsonValue]:
    events: list[JsonValue] = [
        base64.b64encode(chunk).decode("ascii") for chunk in stream
    ]
    return events


async def async_events(stream: AsyncEvents) -> list[JsonValue]:
    events: list[JsonValue] = [
        base64.b64encode(chunk).decode("ascii") async for chunk in stream
    ]
    return events


def sync_unary(case: Case, run: Run, client: HTTPHandler) -> JsonValue:
    generated = invoke(UNARY, case, run.logging_obj, client, is_async=False)
    return unary_answer(run, generated)


def sync_streamed(case: Case, run: Run, client: HTTPHandler) -> JsonValue:
    stream = invoke(STREAM, case, run.logging_obj, client, is_async=False)
    view = views_of(run.logging_obj)
    return stream_answer(run, view, stream, sync_events(stream))


async def async_unary(case: Case, run: Run, client: AsyncHTTPHandler) -> JsonValue:
    generated = await invoke(ASYNC_UNARY, case, run.logging_obj, client, is_async=True)
    return unary_answer(run, generated)


async def async_streamed(case: Case, run: Run, client: AsyncHTTPHandler) -> JsonValue:
    stream = await invoke(ASYNC_STREAM, case, run.logging_obj, client, is_async=True)
    view = views_of(run.logging_obj)
    return stream_answer(run, view, stream, await async_events(stream))


def sync_call(case: Case) -> JsonValue:
    recorder = Recorder(case)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
    run = Run(recorder, logging_object(case))
    if case["stream"]:
        return sync_streamed(case, run, client)
    return sync_unary(case, run, client)


async def async_call(case: Case) -> JsonValue:
    recorder = Recorder(case)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(recorder.handle))
    run = Run(recorder, logging_object(case))
    if case["stream"]:
        return await async_streamed(case, run, client)
    return await async_unary(case, run, client)


def run(input_value: Case) -> JsonValue:
    if input_value["mode"] == "sync":
        return sync_call(input_value)
    return asyncio.run(async_call(input_value))

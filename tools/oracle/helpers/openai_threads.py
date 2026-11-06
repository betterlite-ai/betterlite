import asyncio
import functools
import inspect
import json
import os
from dataclasses import dataclass, field
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm.llms.openai.openai as litellm_openai
import openai
from litellm.llms.openai.openai import OpenAIAssistantsAPI

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

THREAD = '{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"core"}}'
MESSAGE = (
    '{"id":"msg_1","object":"thread.message","created_at":7,"thread_id":"thread_1",'
    '"role":"assistant","content":[{"type":"text","text":{"value":"hi","annotations":[]}}],'
    '"status":"completed"}'
)
PAGE = (
    '{"object":"list","data":['
    + MESSAGE
    + '],"first_id":"msg_1","last_id":"msg_1","has_more":false}'
)
THREAD_ID = "thread_1"
MESSAGE_DATA: dict[str, Json] = {"role": "user", "content": "hi"}


def client_class() -> str:
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return "OpenAI"
    return "AsyncOpenAI"


@dataclass
class Wire:
    bodies: list[str]
    status: int = 200
    seen: list[Json] = field(default_factory=lambda: cast(list[Json], []))
    kinds: list[Json] = field(default_factory=lambda: cast(list[Json], []))
    position: int = 0

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
        self.kinds.append(client_class())
        body = self.bodies[min(self.position, len(self.bodies) - 1)]
        self.position += 1
        return httpx.Response(
            self.status, content=body, headers={"content-type": "application/json"}
        )


@dataclass
class Options:
    api_key: str | None
    api_base: str | None
    timeout: float | None
    max_retries: int | None
    organization: str | None


def options_of(raw: Json) -> Options:
    fields = cast(dict[str, Json], raw or {})
    timeout = fields.get("timeout")
    return Options(
        api_key=cast("str | None", fields.get("api_key")),
        api_base=cast("str | None", fields.get("api_base")),
        timeout=None if timeout is None else float(cast(float, timeout)),
        max_retries=cast("int | None", fields.get("max_retries")),
        organization=cast("str | None", fields.get("organization")),
    )


def wire_for(fields: dict[str, Json], canned: str) -> Wire:
    reply = cast("dict[str, Json] | None", fields.get("reply"))
    if reply is None:
        return Wire([canned])
    return Wire([cast(str, reply["body"])], cast(int, reply["status"]))


def sdk_clients(wire: Wire) -> tuple[object, object]:
    sync = functools.partial(
        openai.OpenAI, http_client=httpx.Client(transport=httpx.MockTransport(wire.handle))
    )
    asynchronous = functools.partial(
        openai.AsyncOpenAI,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire.handle)),
    )
    return sync, asynchronous


def failure(error: Exception) -> Json:
    if isinstance(error, openai.APIStatusError):
        return {"kind": "status", "status": error.status_code}
    if isinstance(error, openai.OpenAIError):
        return {"kind": "configuration"}
    if type(error) is ValueError:
        return {"kind": "invalid_thread_id"}
    raise error


def settle(call: object) -> Json:
    settled = asyncio.run(call) if inspect.iscoroutine(call) else call  # pyright: ignore[reportUnknownArgumentType]
    return cast(Json, cast("openai.BaseModel", settled).model_dump(exclude_none=True))


def create_thread(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = wire_for(fields, THREAD)
    sync, asynchronous = sdk_clients(wire)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = api.create_thread(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                metadata=cast("dict[str, str] | None", fields.get("metadata")),
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                messages=cast("list[Json] | None", fields.get("messages")),  # pyright: ignore[reportArgumentType]
                client=None,
                acreate_thread=fields.get("acreate_thread"),  # pyright: ignore[reportArgumentType]
            )
            return {"requests": wire.seen, "result": settle(call)}  # pyright: ignore[reportUnknownArgumentType]
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": wire.seen, "error": failure(error)}


def get_thread(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = wire_for(fields, THREAD)
    sync, asynchronous = sdk_clients(wire)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = api.get_thread(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                thread_id=cast(str, fields["thread_id"]),
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=None,
                aget_thread=fields.get("aget_thread"),  # pyright: ignore[reportArgumentType]
            )
            return {"requests": wire.seen, "result": settle(call)}  # pyright: ignore[reportUnknownArgumentType]
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": wire.seen, "error": failure(error)}


def add_message(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = wire_for(fields, MESSAGE)
    sync, asynchronous = sdk_clients(wire)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = api.add_message(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                thread_id=cast(str, fields["thread_id"]),
                message_data=cast(dict[str, Json], fields["data"]),
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=None,
                a_add_message=fields.get("a_add_message"),  # pyright: ignore[reportArgumentType]
            )
            return {"requests": wire.seen, "result": settle(call)}  # pyright: ignore[reportUnknownArgumentType]
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": wire.seen, "error": failure(error)}


def get_messages(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = wire_for(fields, PAGE)
    sync, asynchronous = sdk_clients(wire)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = api.get_messages(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                thread_id=cast(str, fields["thread_id"]),
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=None,
                aget_messages=fields.get("aget_messages"),  # pyright: ignore[reportArgumentType]
            )
            return {"requests": wire.seen, "result": settle(call)}  # pyright: ignore[reportUnknownArgumentType]
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": wire.seen, "error": failure(error)}


def dispatch(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = Wire([THREAD, THREAD, MESSAGE, PAGE])
    sync, asynchronous = sdk_clients(wire)
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            thread = settle(
                api.create_thread(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                    metadata=None,
                    api_key=options.api_key,
                    api_base=options.api_base,
                    timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                    max_retries=options.max_retries,
                    organization=options.organization,
                    messages=None,
                    client=None,
                    acreate_thread=fields.get("acreate_thread"),  # pyright: ignore[reportArgumentType]
                )
            )
            retrieved = settle(
                api.get_thread(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                    thread_id=THREAD_ID,
                    api_key=options.api_key,
                    api_base=options.api_base,
                    timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                    max_retries=options.max_retries,
                    organization=options.organization,
                    client=None,
                    aget_thread=fields.get("aget_thread"),  # pyright: ignore[reportArgumentType]
                )
            )
            message = settle(
                api.add_message(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                    thread_id=THREAD_ID,
                    message_data=MESSAGE_DATA,
                    api_key=options.api_key,
                    api_base=options.api_base,
                    timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                    max_retries=options.max_retries,
                    organization=options.organization,
                    client=None,
                    a_add_message=fields.get("a_add_message"),  # pyright: ignore[reportArgumentType]
                )
            )
            page = settle(
                api.get_messages(  # pyright: ignore[reportUnknownMemberType, reportCallIssue, reportUnknownVariableType, reportUnknownArgumentType]
                    thread_id=THREAD_ID,
                    api_key=options.api_key,
                    api_base=options.api_base,
                    timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                    max_retries=options.max_retries,
                    organization=options.organization,
                    client=None,
                    aget_messages=fields.get("aget_messages"),  # pyright: ignore[reportArgumentType]
                )
            )
        except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
            return {"requests": wire.seen, "error": failure(error)}
    return {
        "requests": wire.seen,
        "results": [thread, retrieved, message, page],
        "kinds": wire.kinds,
    }


def run(input_value: dict[str, Json]) -> Json:
    ((variant, fields),) = input_value.items()
    arguments = cast(dict[str, Json], fields)
    match variant:
        case "create_thread":
            return create_thread(arguments)
        case "get_thread":
            return get_thread(arguments)
        case "add_message":
            return add_message(arguments)
        case "get_messages":
            return get_messages(arguments)
        case "dispatch":
            return dispatch(arguments)
        case _:
            raise ValueError(f"unknown variant {variant}")

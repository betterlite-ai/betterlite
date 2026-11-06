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

PAGE = '{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}'
ASSISTANT = (
    '{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[],'
    '"tool_resources":null}'
)
DELETED = '{"id":"asst_1","deleted":true,"object":"assistant.deleted"}'


@dataclass
class Wire:
    status: int
    body: str
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
        return httpx.Response(
            self.status, content=self.body, headers={"content-type": "application/json"}
        )


@dataclass
class Built:
    kind: str
    data: dict[str, Json]


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


def reply_of(fields: dict[str, Json], canned: str) -> Wire:
    reply = cast("dict[str, Json] | None", fields.get("reply"))
    if reply is None:
        return Wire(200, canned)
    return Wire(cast(int, reply["status"]), cast(str, reply["body"]))


def failure(error: Exception) -> Json:
    if isinstance(error, openai.APIStatusError):
        return {"kind": "status", "status": error.status_code}
    if isinstance(error, openai.OpenAIError):
        return {"kind": "configuration"}
    if isinstance(error, ValueError):
        return {"kind": "invalid_assistant_id"}
    raise error


def outcome(wire: Wire, call: object) -> Json:
    try:
        result = asyncio.run(call) if inspect.iscoroutine(call) else call
    except Exception as error:  # noqa: BLE001 - every SDK failure is a reported result
        return {"requests": wire.seen, "error": failure(error)}
    dumped = cast("openai.BaseModel", result).model_dump(exclude_none=True)
    return {"requests": wire.seen, "result": cast(Json, dumped)}


def sdk_clients(wire: Wire) -> tuple[object, object]:
    sync = functools.partial(
        openai.OpenAI, http_client=httpx.Client(transport=httpx.MockTransport(wire.handle))
    )
    asynchronous = functools.partial(
        openai.AsyncOpenAI,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire.handle)),
    )
    return sync, asynchronous


def list_assistants(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = reply_of(fields, PAGE)
    sync, asynchronous = sdk_clients(wire)
    params = cast(dict[str, Json], fields.get("params") or {})
    api = OpenAIAssistantsAPI()
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        if fields.get("direct"):
            call = api.async_get_assistants(
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=None,
                **params,  # pyright: ignore[reportArgumentType]
            )
            return outcome(wire, call)
        try:
            call = api.get_assistants(  # pyright: ignore[reportCallIssue, reportUnknownVariableType]
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=None,
                aget_assistants=fields.get("flag"),  # pyright: ignore[reportArgumentType]
                **params,  # pyright: ignore[reportArgumentType]
            )
        except Exception as error:  # noqa: BLE001 - construction failures are reported results
            return {"requests": wire.seen, "error": failure(error)}
        return outcome(wire, call)  # pyright: ignore[reportUnknownArgumentType]


def create_assistant(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = reply_of(fields, ASSISTANT)
    sync, asynchronous = sdk_clients(wire)
    data = cast("dict[str, object]", fields["data"])
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = OpenAIAssistantsAPI().create_assistants(  # pyright: ignore[reportUnknownMemberType]
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                create_assistant_data=data,
                client=None,
                async_create_assistants=fields.get("flag"),
            )
        except Exception as error:  # noqa: BLE001 - construction failures are reported results
            return {"requests": wire.seen, "error": failure(error)}
        return outcome(wire, call)


def delete_assistant(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    wire = reply_of(fields, DELETED)
    sync, asynchronous = sdk_clients(wire)
    with (
        patch.dict(os.environ, {}, clear=True),
        patch.object(litellm_openai, "OpenAI", sync),
        patch.object(litellm_openai, "AsyncOpenAI", asynchronous),
    ):
        try:
            call = OpenAIAssistantsAPI().delete_assistant(  # pyright: ignore[reportUnknownMemberType]
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                assistant_id=cast(str, fields["assistant_id"]),
                client=None,
                async_delete_assistants=fields.get("flag"),
            )
        except Exception as error:  # noqa: BLE001 - construction failures are reported results
            return {"requests": wire.seen, "error": failure(error)}
        return outcome(wire, call)


@dataclass
class Recorder:
    kind: str

    def __call__(self, **data: Json) -> Built:
        return Built(self.kind, data)


def client_arguments(fields: dict[str, Json]) -> Json:
    options = options_of(fields.get("options"))
    kind = cast(str, fields["kind"])
    supplied = cast("dict[str, Json] | None", fields.get("supplied"))
    api = OpenAIAssistantsAPI()
    client = None
    if supplied is not None:
        organization = supplied.get("organization")
        client = Built(
            cast(str, supplied["kind"]),
            {} if organization is None else {"organization": organization},
        )
    with (
        patch.object(litellm_openai, "OpenAI", Recorder("sync")),
        patch.object(litellm_openai, "AsyncOpenAI", Recorder("async")),
    ):
        build = api.get_openai_client if kind == "sync" else api.async_get_openai_client
        built = cast(
            Built,
            build(  # pyright: ignore[reportCallIssue]
                api_key=options.api_key,
                api_base=options.api_base,
                timeout=options.timeout,  # pyright: ignore[reportArgumentType]
                max_retries=options.max_retries,
                organization=options.organization,
                client=client,  # pyright: ignore[reportArgumentType]
            ),
        )
    return {"client": {"kind": built.kind, **built.data}}


def run(input_value: dict[str, Json]) -> Json:
    ((variant, fields),) = input_value.items()
    arguments = cast(dict[str, Json], fields)
    match variant:
        case "list_assistants":
            return list_assistants(arguments)
        case "create_assistant":
            return create_assistant(arguments)
        case "delete_assistant":
            return delete_assistant(arguments)
        case "client_arguments":
            return client_arguments(arguments)
        case _:
            raise ValueError(f"unknown variant {variant}")

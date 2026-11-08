import asyncio
import json
import os
from collections.abc import Generator
from contextlib import ExitStack, contextmanager
from typing import Literal, Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.secret_managers import aws_secret_manager_v2 as aws

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Path: TypeAlias = Literal["async", "sync"]
FIELDS = {"env", "routes", "steps"}


class Manager(Protocol):
    async def async_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, Json] | None,
        request_timeout: None,
        primary_secret_name: str | None,
    ) -> str | None: ...

    def sync_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, Json] | None,
        request_timeout: None,
        primary_secret_name: str | None,
    ) -> str | None: ...

    async def async_read_secret_from_primary_secret(
        self, secret_name: str, primary_secret_name: str
    ) -> str | None: ...

    def sync_read_secret_from_primary_secret(
        self, secret_name: str, primary_secret_name: str
    ) -> str | None: ...


def as_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise ValueError("expected an object")
    return value


def as_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("expected a list")
    return value


def as_text(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError("expected a string")
    return value


def as_int(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("expected an integer")
    return value


def as_path(value: Json) -> Path:
    match as_text(value):
        case "async":
            return "async"
        case "sync":
            return "sync"
        case other:
            raise ValueError(f"unknown b4414 path: {other}")


def optional_string(value: Json) -> str | None:
    return None if value is None else as_text(value)


def optional_object(value: Json) -> Object | None:
    return None if value is None else as_object(value)


class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = as_object(entry)
            self.replies[as_text(fields["route"])] = [
                as_object(reply) for reply in as_list(fields["replies"])
            ]
        self.requests: list[Json] = []

    def next_reply(self, route: str) -> Object:
        replies = self.replies.get(route, [])
        if len(replies) > 1:
            return replies.pop(0)
        if replies:
            return replies[0]
        return {"status": 500, "body": ""}

    def answer(self, request: httpx.Request, *, blocking: bool) -> httpx.Response:
        content = request.content
        body: Json = cast(Json, json.loads(content)) if content else None
        self.requests.append(
            {
                "url": str(request.url),
                "target": request.headers.get("x-amz-target"),
                "body": body,
                "blocking": blocking,
            }
        )
        reply = self.next_reply(f"POST {request.url}")
        if reply.get("timeout") is True:
            raise httpx.ReadTimeout("the mock timed out", request=request)
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused", request=request)
        return httpx.Response(
            as_int(reply["status"]), text=as_text(reply["body"]), request=request
        )


def mock_for(transport: Transport, *, blocking: bool) -> httpx.MockTransport:
    def handle(request: httpx.Request) -> httpx.Response:
        return transport.answer(request, blocking=blocking)

    return httpx.MockTransport(handle)


@contextmanager
def patched_world(env: Object, transport: Transport) -> Generator[None]:
    saved_environment = {
        name: value for name, value in os.environ.items() if name.startswith("AWS_")
    }
    saved_client = litellm.secret_manager_client
    saved_system = litellm._key_management_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    for name in saved_environment:
        del os.environ[name]
    for name, value in env.items():
        os.environ[name] = as_text(value)

    def sync_handler(**options: Json) -> HTTPHandler:
        del options
        return HTTPHandler(client=httpx.Client(transport=mock_for(transport, blocking=True)))

    def async_handler(**options: Json) -> AsyncHTTPHandler:
        del options
        return AsyncHTTPHandler(transport=mock_for(transport, blocking=False))

    def no_native(*_arguments: object) -> None:
        pass

    try:
        litellm.secret_manager_client = None
        with ExitStack() as stack:
            stack.enter_context(patch.object(aws, "_get_httpx_client", sync_handler))
            stack.enter_context(patch.object(aws, "get_async_httpx_client", async_handler))
            stack.enter_context(
                patch.object(aws, "resolve_native_provider_reader", no_native)
            )
            yield
    finally:
        for name in [name for name in os.environ if name.startswith("AWS_")]:
            del os.environ[name]
        os.environ.update(saved_environment)
        litellm.secret_manager_client = saved_client
        litellm._key_management_system = saved_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


async def read(
    manager: Manager, path: Path, secret: str, primary: str | None, params: Object | None
) -> Json:
    try:
        if path == "async":
            value = await manager.async_read_secret(secret, params, None, primary)
        else:
            value = manager.sync_read_secret(secret, params, None, primary)
    except Exception:  # noqa: BLE001 - a case compares only that the read raised
        return {"raised": True}
    else:
        return {"value": value}


async def read_primary(manager: Manager, path: Path, secret: str, primary: str) -> Json:
    try:
        if path == "async":
            value = await manager.async_read_secret_from_primary_secret(secret, primary)
        else:
            value = manager.sync_read_secret_from_primary_secret(secret, primary)
    except Exception:  # noqa: BLE001 - a case compares only that the read raised
        return {"raised": True}
    else:
        return {"value": value}


async def run_step(manager: Manager, step: Object) -> Json:
    op = as_text(step["op"])
    path = as_path(step["path"])
    secret = as_text(step["secret"])
    if op == "read":
        primary = optional_string(step.get("primary"))
        params = optional_object(step.get("params"))
        return await read(manager, path, secret, primary, params)
    if op == "read_primary":
        return await read_primary(manager, path, secret, as_text(step["primary"]))
    raise ValueError(f"unknown b4414 step: {op}")


async def execute(input_value: Object, transport: Transport) -> Json:
    manager = cast(Manager, aws.AWSSecretsManagerV2())
    steps: list[Json] = []
    for step in as_list(input_value["steps"]):
        result = await run_step(manager, as_object(step))
        steps.append({"result": result, "requests": transport.requests})
        transport.requests = []
    return {"steps": steps}


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    transport = Transport(as_list(input_value["routes"]))
    with patched_world(as_object(input_value["env"]), transport):
        return asyncio.run(execute(input_value, transport))

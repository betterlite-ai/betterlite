import asyncio
import json
import os
import uuid
from collections.abc import Generator
from contextlib import ExitStack, contextmanager
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.secret_managers import aws_secret_manager_v2 as aws

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "manager", "routes", "steps"}
MANAGER_FIELDS = {"aws_region_name", "replica_regions", "kms_key_id"}
AWS_VARIABLES = ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_REGION_NAME", "AWS_REGION")


class Manager(Protocol):
    async def async_write_secret(
        self,
        secret_name: str,
        secret_value: str,
        description: str | None = None,
        tags: Json = None,
    ) -> Object: ...

    async def async_delete_secret(
        self,
        secret_name: str,
        recovery_window_in_days: int | None = 7,
    ) -> Object: ...

    async def async_rotate_secret(
        self,
        current_secret_name: str,
        new_secret_name: str,
        new_secret_value: str,
    ) -> Object: ...

    async def async_replicate_secret(
        self,
        secret_name: str,
        replica_regions: list[str],
    ) -> Object: ...

    async def async_put_secret_value(
        self,
        secret_name: str,
        secret_value: str,
    ) -> Object: ...


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


def optional_string(value: Json) -> str | None:
    return None if value is None else as_text(value)


def optional_object(value: Json) -> Object | None:
    return None if value is None else as_object(value)


def is_uuid(value: str) -> bool:
    try:
        uuid.UUID(value)
    except ValueError:
        return False
    return True




class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = as_object(entry)
            self.replies[as_text(fields["route"])] = [
                as_object(reply) for reply in as_list(fields["replies"])
            ]
        self.requests: list[Json] = []
        self.tokens: dict[str, str] = {}

    def placeholder(self, token: str) -> str:
        if token not in self.tokens:
            self.tokens[token] = f"<uuid-{len(self.tokens) + 1}>"
        return self.tokens[token]

    def normalized(self, body: Json) -> Json:
        if isinstance(body, dict):
            token = body.get("ClientRequestToken")
            if isinstance(token, str) and is_uuid(token):
                body["ClientRequestToken"] = self.placeholder(token)
        return body

    def next_reply(self, route: str) -> Object:
        replies = self.replies.get(route, [])
        if len(replies) > 1:
            return replies.pop(0)
        if replies:
            return replies[0]
        return {"status": 500, "body": ""}

    def answer(self, request: httpx.Request) -> httpx.Response:
        content = request.content
        body: Json = cast(Json, json.loads(content)) if content else None
        self.requests.append(
            {
                "url": str(request.url),
                "target": request.headers.get("x-amz-target"),
                "body": self.normalized(body),
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


def build_manager(settings: Object) -> Manager:
    regions = [as_text(region) for region in as_list(settings.get("replica_regions") or [])]
    client = aws.AWSSecretsManagerV2(
        aws_region_name=optional_string(settings.get("aws_region_name")),
        replica_regions=regions,
        kms_key_id=optional_string(settings.get("kms_key_id")),
    )
    return cast(Manager, client)


@contextmanager
def patched_world(env: Object, transport: Transport) -> Generator[None]:
    saved_environment = {
        name: value for name, value in os.environ.items() if name.startswith("AWS_")
    }
    saved_client = litellm.secret_manager_client
    for name in saved_environment:
        del os.environ[name]
    for name, value in env.items():
        os.environ[name] = as_text(value)

    def async_handler(**_options: Json) -> AsyncHTTPHandler:
        return AsyncHTTPHandler(transport=httpx.MockTransport(transport.answer))

    def no_native(*_arguments: object) -> None:
        pass

    try:
        litellm.secret_manager_client = None
        with ExitStack() as stack:
            stack.enter_context(patch.object(aws, "get_async_httpx_client", async_handler))
            stack.enter_context(patch.object(aws, "resolve_native_provider_reader", no_native))
            yield
    finally:
        for name in [name for name in os.environ if name.startswith("AWS_")]:
            del os.environ[name]
        os.environ.update(saved_environment)
        litellm.secret_manager_client = saved_client


async def run_step(manager: Manager, step: Object) -> Json:
    operation = as_text(step["op"])
    try:
        match operation:
            case "async_write_secret":
                result = await manager.async_write_secret(
                    secret_name=as_text(step["name"]),
                    secret_value=as_text(step["value"]),
                    description=optional_string(step.get("description")),
                    tags=step.get("tags"),
                )
            case "async_delete_secret":
                result = await manager.async_delete_secret(
                    secret_name=as_text(step["name"]),
                    recovery_window_in_days=as_int(step.get("recovery_window_in_days", 7)),
                )
            case "async_rotate_secret":
                result = await manager.async_rotate_secret(
                    current_secret_name=as_text(step["current"]),
                    new_secret_name=as_text(step["new"]),
                    new_secret_value=as_text(step["value"]),
                )
            case "async_replicate_secret":
                result = await manager.async_replicate_secret(
                    secret_name=as_text(step["name"]),
                    replica_regions=[
                        as_text(region) for region in as_list(step.get("regions") or [])
                    ],
                )
            case "async_put_secret_value":
                result = await manager.async_put_secret_value(
                    secret_name=as_text(step["name"]),
                    secret_value=as_text(step["value"]),
                )
            case _:
                raise ValueError(f"unknown b4415 step: {operation}")
    except Exception:  # noqa: BLE001 - a case compares only that the call raised
        return {"raised": True}
    return {"body": result}


async def execute(input_value: Object, transport: Transport) -> Json:
    manager = build_manager(as_object(input_value["manager"]))
    steps: list[Json] = []
    for step in as_list(input_value["steps"]):
        result = await run_step(manager, as_object(step))
        steps.append({"result": result, "requests": transport.requests})
        transport.requests = []
    return {"steps": steps}


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    settings = as_object(input_value["manager"])
    if set(settings) - MANAGER_FIELDS:
        raise ValueError(f"unknown manager settings: {sorted(set(settings) - MANAGER_FIELDS)}")
    transport = Transport(as_list(input_value["routes"]))
    with patched_world(as_object(input_value["env"]), transport):
        return asyncio.run(execute(input_value, transport))
